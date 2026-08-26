use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use vecdb_common::output::OutputFormat;
use vecdb_core::config::Config;
use vecdb_core::config::QuantizationType;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecdb_core::types::{CollectionGenesis, CollectionInfo};
use vecq::detection::HybridDetector;

pub async fn run(
    config: &Config,
    profile_name: Option<&str>,
    format: OutputFormat,
) -> anyhow::Result<()> {
    // Performance: Avoid connecting to Ollama/loading local model just to list collections
    std::env::set_var("VECDB_SKIP_PROBE", "true");

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    // One resolution per distinct Qdrant endpoint. Listing is about *stores*,
    // not embedders, so profiles that differ only in which model they use would
    // enumerate the same collections twice.
    let mut to_check: Vec<vecdb_core::config::Resolution> = Vec::new();

    if let Some(p) = profile_name {
        to_check.push(config.resolve(Some(p), None)?);
    } else {
        let mut seen = std::collections::HashSet::new();
        let default_res = config.resolve(None, None)?;
        seen.insert(default_res.qdrant_url.clone());
        to_check.push(default_res);

        for name in config.profiles.keys() {
            if let Ok(r) = config.resolve(Some(name), None) {
                if seen.insert(r.qdrant_url.clone()) {
                    to_check.push(r);
                }
            }
        }

        // A collection may point at a Qdrant no profile mentions.
        for col_name in config.collections.keys() {
            if let Ok(r) = config.resolve(None, Some(col_name)) {
                if seen.insert(r.qdrant_url.clone()) {
                    to_check.push(r);
                }
            }
        }
    }

    let mut results: Vec<(String, Vec<(CollectionInfo, CollectionGenesis)>)> = Vec::new();

    let mut unreachable_backends: Vec<(String, String)> = Vec::new();

    for resolution in to_check {
        let services = vecdb_core::CoreServices::from_config(
            config,
            file_detector.clone(),
            parser_factory.clone(),
        );
        let core = vecdb_core::Core::new(&resolution, services).await?;

        // A backend that cannot be reached is recorded, never swallowed.
        //
        // This previously warned on stderr for human output and said nothing at
        // all under --json, then exited 0 with an empty object. An unreachable
        // Qdrant was therefore indistinguishable from an empty one: the caller
        // reads "no collections", concludes the database is empty, and stops.
        // That is the most expensive wrong answer this command can give.
        match core.list_collections_with_genesis().await {
            Ok(cols) => {
                results.push((resolution.qdrant_url, cols));
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to list collections for {}: {}",
                    resolution.qdrant_url, e
                );
                unreachable_backends.push((resolution.qdrant_url, e.to_string()));
            }
        }
    }

    // Captured before `results` is consumed by the human-output branch below.
    let nothing_listed = results.is_empty();

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    match format {
        OutputFormat::Json => {
            // Depending on if a single profile was requested, we might just output a flat array
            // to preserve backward compatibility for scripts, or map by backend.
            // `is_vecdb` is emitted explicitly so a consumer never has to infer
            // ownership from the presence or absence of other fields.
            let encode = |cols: &[(CollectionInfo, CollectionGenesis)]| {
                cols.iter()
                    .map(|(c, g)| {
                        serde_json::json!({
                            "name": c.name,
                            "vector_count": c.vector_count,
                            "vector_size": c.vector_size,
                            "quantization": c.quantization,
                            "vectors_on_disk": c.vectors_on_disk,
                            "payload_on_disk": c.payload_on_disk,
                            "is_vecdb": g.is_vecdb(),
                            "vecdb_version": g.vecdb_version,
                            // The build, not just the release. Between releases
                            // the version is constant while semantics change.
                            "vecdb_revision": g.vecdb_revision,
                            "model": g.is_vecdb().then(|| g.model.clone()),
                            // null for collections created before this was
                            // recorded — absence is itself the answer to
                            // "how was this chunked?", not a missing field.
                            "chunking": g.chunking.clone(),
                            "created_at": g.created_at,
                        })
                    })
                    .collect::<Vec<_>>()
            };

            if profile_name.is_some() {
                match results.first() {
                    Some((_, cols)) => serde_json::to_writer(&mut out, &encode(cols))?,
                    None => serde_json::to_writer(&mut out, &Vec::<serde_json::Value>::new())?,
                }
            } else {
                let mut all_cols = HashMap::new();
                for (url, cols) in &results {
                    all_cols.insert(url.clone(), encode(cols));
                }
                // Unreachable backends appear as an explicit error entry rather
                // than as a missing key, so "this backend is down" cannot be
                // read as "this backend has no collections".
                for (url, err) in &unreachable_backends {
                    all_cols.insert(
                        url.clone(),
                        vec![serde_json::json!({ "error": err, "unreachable": true })],
                    );
                }
                serde_json::to_writer(&mut out, &all_cols)?;
            }
            writeln!(out)?;
        }
        _ => {
            if results.is_empty() {
                if unreachable_backends.is_empty() {
                    writeln!(out, "No collections found across any backend.")?;
                } else {
                    writeln!(
                        out,
                        "No backend could be reached, so nothing could be listed."
                    )?;
                    for (url, err) in &unreachable_backends {
                        writeln!(out, "  {url}: {err}")?;
                    }
                }
            } else {
                for (url, collections) in results {
                    let is_local = url.contains("localhost")
                        || url.contains("127.0.0.1")
                        || url.contains("0.0.0.0");
                    let location_tag = if is_local { "Local" } else { "Remote" };
                    writeln!(out, "\nBackend: {} ({})", url, location_tag)?;

                    if collections.is_empty() {
                        writeln!(out, "  No collections found.")?;
                        continue;
                    }

                    writeln!(
                        out,
                        "  {:<20} | {:<12} | {:<6} | {:<8} | Model",
                        "Name", "Vectors", "Dim", "Quant"
                    )?;
                    writeln!(
                        out,
                        "  {:-<20}-+-{:-<12}-+-{:-<6}-+-{:-<8}-+-{:-<30}",
                        "", "", "", "", ""
                    )?;
                    let mut foreign = 0usize;
                    for (c, genesis) in collections {
                        let count_val = c.vector_count.unwrap_or(0);
                        let dim_val = c.vector_size.unwrap_or(0);
                        let (bytes_per_dim, overhead_mult) = match c.quantization {
                            Some(QuantizationType::Scalar) => (1.0, 1.2),
                            Some(QuantizationType::Binary) => (0.125, 1.2),
                            _ => (4.0, 1.2),
                        };

                        let total_bytes =
                            (count_val as f64 * dim_val as f64 * bytes_per_dim) * overhead_mult;

                        // Adjust for on-disk storage: if vectors are on disk, they do not consume RAM
                        let vectors_on_disk = c.vectors_on_disk.unwrap_or(false);
                        let adjusted_bytes = if vectors_on_disk {
                            // Vectors on disk = minimal RAM usage (just metadata/index)
                            // Estimate ~1% of original for index overhead
                            total_bytes * 0.01
                        } else {
                            total_bytes
                        };

                        let size_gb = adjusted_bytes / (1024.0 * 1024.0 * 1024.0);
                        let count = c
                            .vector_count
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        let dim = c
                            .vector_size
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".to_string());
                        let quant = match c.quantization {
                            Some(QuantizationType::Scalar) => "Scalar",
                            Some(QuantizationType::Binary) => "Binary",
                            _ => "None",
                        };

                        // Foreign collections are listed, never hidden — a name
                        // that is absent here but rejects an ingest is a support
                        // ticket. The Model column carries the label because
                        // "which model is this?" and "is this ours?" are the
                        // same question.
                        let model = if genesis.is_vecdb() {
                            genesis.model.describe()
                        } else {
                            foreign += 1;
                            "— not a vecdb collection".to_string()
                        };

                        writeln!(
                            out,
                            "  {:<20} | {:<12} | {:<6} | {:<8} | {}",
                            c.name, count, dim, quant, model
                        )?;
                        if size_gb > 4.0 {
                            if matches!(
                                c.quantization,
                                Some(QuantizationType::Scalar) | Some(QuantizationType::Binary)
                            ) {
                                writeln!(
                                    out,
                                    "    ^-- NOTE: Approx {:.2} GB RAM (Optimized).",
                                    size_gb
                                )?;
                            } else {
                                writeln!(
                                    out,
                                    "    ^-- WARNING: Approx {:.2} GB RAM. Consider 'vecdb optimize {}'",
                                    size_gb, c.name
                                )?;
                            }
                        }
                    }
                    if foreign > 0 {
                        writeln!(
                            out,
                            "\n  {} collection(s) on this backend were not created by vecdb.\n  \
                             They are shown for visibility; vecdb will not read or write them.",
                            foreign
                        )?;
                    }
                }
                writeln!(out)?;
            }
        }
    }
    out.flush()?;

    // Exit non-zero when nothing could be listed and at least one backend was
    // unreachable. A script or agent that checks the status code must be able
    // to tell "the database is empty" (success, empty output) from "I could not
    // ask" (failure). Partial success — some backends reachable — still exits 0
    // with the failures reported alongside the results.
    if nothing_listed && !unreachable_backends.is_empty() {
        anyhow::bail!(
            "could not reach any configured backend ({})",
            unreachable_backends
                .iter()
                .map(|(url, _)| url.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}
