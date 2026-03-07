use crate::vecq_adapter::VecqParserFactory;
use std::io::Write;
use std::sync::Arc;
use vecdb_common::output::OutputFormat;
use vecdb_core::config::Config;
use vecdb_core::config::QuantizationType;
use vecdb_core::types::CollectionInfo;
use vecq::detection::HybridDetector;
use std::collections::HashMap;

pub async fn run(config: &Config, profile_name: Option<&str>, format: OutputFormat) -> anyhow::Result<()> {
    // Performance: Avoid connecting to Ollama/loading local model just to list collections
    std::env::set_var("VECDB_SKIP_PROBE", "true");

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let mut profiles_to_check = Vec::new();
    
    if let Some(p) = profile_name {
        profiles_to_check.push(config.resolve_profile(Some(p), None)?);
    } else {
        // Collect all unique Qdrant URLs, ensuring default profile is first
        let default_prof = config.resolve_profile(None, None)?;
        let mut seen = std::collections::HashSet::new();
        seen.insert(default_prof.qdrant_url.clone());
        profiles_to_check.push(default_prof);

        for name in config.profiles.keys() {
            if let Ok(prof) = config.resolve_profile(Some(name), None) {
                if !seen.contains(&prof.qdrant_url) {
                    seen.insert(prof.qdrant_url.clone());
                    profiles_to_check.push(prof);
                }
            }
        }
    }

    let mut results: Vec<(String, Vec<CollectionInfo>)> = Vec::new();

    for profile in profiles_to_check {
        let core = vecdb_core::Core::new(
            &profile.qdrant_url,
            &profile.ollama_url,
            &config.resolve_embedding_model(&profile),
            profile.accept_invalid_certs,
            &profile.embedder_type,
            Some(config.fastembed_cache_path.clone()),
            config.resolve_local_use_gpu(None),
            profile.qdrant_api_key.clone(),
            profile.ollama_api_key.clone(),
            config.smart_routing_keys.clone(),
            config.ingestion.path_rules.clone(),
            config.ingestion.max_concurrent_requests,
            config.resolve_gpu_batch_size(&profile, None),
            profile.num_ctx,
            file_detector.clone(),
            parser_factory.clone(),
        )
        .await?;

        // We only append to results if it connects successfully, log warning otherwise
        match core.list_collections().await {
            Ok(cols) => {
                results.push((profile.qdrant_url, cols));
            }
            Err(e) => {
                if format != OutputFormat::Json {
                    eprintln!("Warning: Failed to list collections for {}: {}", profile.qdrant_url, e);
                }
            }
        }
    }

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    match format {
        OutputFormat::Json => {
            // Depending on if a single profile was requested, we might just output a flat array 
            // to preserve backward compatibility for scripts, or map by backend.
            if profile_name.is_some() {
                if let Some((_, cols)) = results.first() {
                    serde_json::to_writer(&mut out, cols)?;
                } else {
                    serde_json::to_writer(&mut out, &Vec::<CollectionInfo>::new())?;
                }
            } else {
                let mut all_cols = HashMap::new();
                for (url, cols) in results {
                    all_cols.insert(url, cols);
                }
                serde_json::to_writer(&mut out, &all_cols)?;
            }
            writeln!(out)?;
        }
        _ => {
            if results.is_empty() {
                writeln!(out, "No collections found across any backend.")?;
            } else {
                for (url, collections) in results {
                    let is_local = url.contains("localhost") || url.contains("127.0.0.1") || url.contains("0.0.0.0");
                    let location_tag = if is_local { "Local" } else { "Remote" };
                    writeln!(out, "\nBackend: {} ({})", url, location_tag)?;
                    
                    if collections.is_empty() {
                        writeln!(out, "  No collections found.")?;
                        continue;
                    }

                    writeln!(
                        out,
                        "  {:<20} | {:<15} | {:<10} | {:<10}",
                        "Name", "Vectors", "Dim", "Quant"
                    )?;
                    writeln!(out, "  {:-<20}-+-{:-<15}-+-{:-<10}-+-{:-<10}", "", "", "", "")?;
                    for c in collections {
                        let count_val = c.vector_count.unwrap_or(0);
                        let dim_val = c.vector_size.unwrap_or(0);
                        let (bytes_per_dim, overhead_mult) = match c.quantization {
                            Some(QuantizationType::Scalar) => (1.0, 1.2),
                            Some(QuantizationType::Binary) => (0.125, 1.2),
                            _ => (4.0, 1.2),
                        };

                        let total_bytes =
                            (count_val as f64 * dim_val as f64 * bytes_per_dim) * overhead_mult;
                        let size_gb = total_bytes / (1024.0 * 1024.0 * 1024.0);

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

                        writeln!(
                            out,
                            "  {:<20} | {:<15} | {:<10} | {:<10}",
                            c.name, count, dim, quant
                        )?;
                        if size_gb > 4.0 {
                            if matches!(
                                c.quantization,
                                Some(QuantizationType::Scalar) | Some(QuantizationType::Binary)
                            ) {
                                writeln!(out, "    ^-- NOTE: Approx {:.2} GB RAM (Optimized).", size_gb)?;
                            } else {
                                writeln!(
                                    out,
                                    "    ^-- WARNING: Approx {:.2} GB RAM. Consider 'vecdb optimize {}'",
                                    size_gb, c.name
                                )?;
                            }
                        }
                    }
                }
                writeln!(out)?;
            }
        }
    }
    out.flush()?;
    Ok(())
}