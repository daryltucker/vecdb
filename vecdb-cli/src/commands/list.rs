use std::collections::HashMap;
use std::io::Write;
use vecdb_common::output::OutputFormat;
use vecdb_core::config::Config;
use vecdb_core::config::QuantizationType;
use vecdb_core::types::{CollectionGenesis, CollectionInfo};

pub async fn run(
    config: &Config,
    profile_name: Option<&str>,
    format: OutputFormat,
) -> anyhow::Result<()> {
    // Every Qdrant the config can name, once each. Listing is about *stores*,
    // not embedders: no embedder is built or announced here — `vecdb list` used
    // to construct a whole `Core` per store, which picked the default profile's
    // embedder and printed "Using embedder …" once per store for a command that
    // embeds nothing (operator, 2026-314).
    //
    // Without a profile: the default resolution's store first, then EVERY
    // `[store.*]` entry — including one no profile or collection references yet
    // (it was invisible before) — then any inline `qdrant_url` a profile or
    // collection sets. A URL reached both ways is listed once, under its store
    // name.
    let mut targets: Vec<Target> = Vec::new();
    let mut add = |url: String, api_key: Option<String>, store: Option<String>| match targets
        .iter_mut()
        .find(|t| t.url == url)
    {
        Some(t) => {
            if t.store.is_none() {
                t.store = store;
            }
        }
        None => targets.push(Target {
            url,
            api_key,
            store,
        }),
    };

    if let Some(p) = profile_name {
        let r = config.resolve(Some(p), None)?;
        add(r.qdrant_url, r.qdrant_api_key, r.store);
    } else {
        let r = config.resolve(None, None)?;
        add(r.qdrant_url, r.qdrant_api_key, r.store);

        let mut stores: Vec<_> = config.store.iter().collect();
        stores.sort_by(|a, b| a.0.cmp(b.0));
        for (name, s) in stores {
            add(s.url.clone(), s.api_key.clone(), Some(name.clone()));
        }

        for name in config.profiles.keys() {
            if let Ok(r) = config.resolve(Some(name), None) {
                add(r.qdrant_url, r.qdrant_api_key, r.store);
            }
        }
        // A collection may point at a Qdrant no profile mentions.
        for col_name in config.collections.keys() {
            if let Ok(r) = config.resolve(None, Some(col_name)) {
                add(r.qdrant_url, r.qdrant_api_key, r.store);
            }
        }
    }

    // (url, configured store name, its collections)
    type Listed = (
        String,
        Option<String>,
        Vec<(CollectionInfo, CollectionGenesis)>,
    );
    let mut results: Vec<Listed> = Vec::new();

    let mut unreachable_backends: Vec<(String, Option<String>, String)> = Vec::new();

    for target in targets {
        // A backend that cannot be reached is recorded, never swallowed.
        //
        // This previously warned on stderr for human output and said nothing at
        // all under --json, then exited 0 with an empty object. An unreachable
        // Qdrant was therefore indistinguishable from an empty one: the caller
        // reads "no collections", concludes the database is empty, and stops.
        // That is the most expensive wrong answer this command can give.
        match vecdb_core::list_store_collections(&target.url, target.api_key.clone()).await {
            Ok(cols) => {
                // `vecdb list` lists vecdb's collections, and nothing else.
                //
                // Foreign collections were previously shown with a label, on the
                // reasoning that a taken name should never be invisible. But the
                // taken name is already declared, loudly and at the only moment
                // it matters: creating or writing one is refused by the genesis
                // ownership check. Listing them a second time here just puts
                // another tool's data in vecdb's inventory.
                let mine: Vec<_> = cols.into_iter().filter(|(_, g)| g.is_vecdb()).collect();
                results.push((target.url, target.store, mine));
            }
            Err(e) => {
                // One readable line, not the transport's error chain. A
                // configured local Qdrant that is simply switched off is
                // normal — some users never run one, others only sometimes —
                // and a `tonic::transport::Error(… ConnectError …)` dump read
                // as a crash (operator, 2026-314). The raw error is kept in
                // `--json` output (`error`) for anything that needs it.
                eprintln!(
                    "Warning: {}",
                    unreachable_message(&target.url, target.store.as_deref(), &e.to_string())
                );
                unreachable_backends.push((target.url, target.store, e.to_string()));
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
                    Some((_, _, cols)) => serde_json::to_writer(&mut out, &encode(cols))?,
                    None => serde_json::to_writer(&mut out, &Vec::<serde_json::Value>::new())?,
                }
            } else {
                let mut all_cols = HashMap::new();
                for (url, _, cols) in &results {
                    all_cols.insert(url.clone(), encode(cols));
                }
                // Unreachable backends appear as an explicit error entry rather
                // than as a missing key, so "this backend is down" cannot be
                // read as "this backend has no collections".
                for (url, store, err) in &unreachable_backends {
                    all_cols.insert(
                        url.clone(),
                        vec![serde_json::json!({
                            "error": err,
                            "unreachable": true,
                            "store": store,
                            "reason": Unreachable::classify(err).as_str(),
                        })],
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
                    for (url, store, err) in &unreachable_backends {
                        writeln!(out, "  {}", unreachable_message(url, store.as_deref(), err))?;
                    }
                }
            } else {
                for (url, store, collections) in results {
                    // Named by its `[store.*]` entry, as configured. This said
                    // "Local"/"Remote" by matching `localhost` in the URL — a
                    // guess that called every LAN store "Remote" (2026-314).
                    match &store {
                        Some(name) => writeln!(out, "\nBackend: {url} (store '{name}')")?,
                        None => writeln!(out, "\nBackend: {url} (inline qdrant_url)")?,
                    }

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

                        // Every row here is vecdb's; foreign collections were
                        // filtered out at the source.
                        let model = genesis.model.describe();

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
                .map(|(url, _, _)| url.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}

/// Why a backend could not be listed, as far as the error says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unreachable {
    /// Nothing is listening: the Qdrant is not running (or the port is wrong).
    Refused,
    /// The host name does not resolve.
    Dns,
    /// The host did not answer in time.
    Timeout,
    /// Connected, but the request failed — reported as-is.
    Other,
}

impl Unreachable {
    /// Read from the error text: the transport error is a nested chain
    /// (`tonic::transport::Error(Transport, ConnectError(…))`) with no stable
    /// type reachable through `anyhow`, but its words are stable.
    fn classify(err: &str) -> Self {
        let e = err.to_ascii_lowercase();
        if e.contains("connection refused") {
            Self::Refused
        } else if e.contains("dns error") || e.contains("failed to lookup address") {
            Self::Dns
        } else if e.contains("timed out") || e.contains("deadline has elapsed") {
            Self::Timeout
        } else {
            Self::Other
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Refused => "connection_refused",
            Self::Dns => "dns",
            Self::Timeout => "timeout",
            Self::Other => "error",
        }
    }
}

/// One Qdrant to list: its URL, its key, and the `[store.*]` name that
/// declares it (`None` for an inline `qdrant_url`).
struct Target {
    url: String,
    api_key: Option<String>,
    store: Option<String>,
}

/// `localhost:6334` from `http://localhost:6334/`.
fn host_port(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// The one-line, human account of an unreachable store:
/// `Could not connect to store 'local' (localhost:6334): connection refused — is it running? Its collections are not listed.`
///
/// Named by its `[store.*]` entry — the name the user gave it in config — not
/// by guessing "local" or "remote" from the URL, which says nothing about a
/// LAN host, an alias, or `::1`. An inline `qdrant_url` has no name and says
/// so.
fn unreachable_message(url: &str, store: Option<&str>, err: &str) -> String {
    let at = host_port(url);
    let which = match store {
        Some(name) => format!("store '{name}' ({at})"),
        None => format!("the Qdrant at {at} (inline qdrant_url)"),
    };
    let why = match Unreachable::classify(err) {
        Unreachable::Refused => "connection refused — is it running?",
        Unreachable::Dns => "the host name does not resolve",
        Unreachable::Timeout => "no answer in time",
        Unreachable::Other => return format!("Could not list collections on {which}: {err}"),
    };
    format!("Could not connect to {which}: {why} Its collections are not listed.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REGRESSION (operator, 2026-314): a switched-off Qdrant printed the whole
    /// tonic transport error chain. It must read as one line naming the store
    /// as configured, its host and port, and the likely cause.
    #[test]
    fn refused_connection_is_one_readable_line_naming_the_store() {
        let err = "Error in the response: Internal error Failed to connect to http://localhost:6334/: \
                   tonic::transport::Error(Transport, ConnectError(ConnectError(\"tcp connect error\", \
                   127.0.0.1:6334, Os { code: 111, kind: ConnectionRefused, message: \"Connection refused\" }))) \
                   MetadataMap { headers: {} }";
        let msg = unreachable_message("http://localhost:6334", Some("local"), err);
        assert_eq!(
            msg,
            "Could not connect to store 'local' (localhost:6334): connection refused — is it running? Its collections are not listed."
        );
        assert!(!msg.contains("tonic"));
        assert_eq!(Unreachable::classify(err).as_str(), "connection_refused");
    }

    #[test]
    fn unnamed_stores_and_other_causes() {
        let url = "http://qdrant.example.com:6334/";
        assert!(unreachable_message(url, None, "dns error: failed to lookup address information")
            .starts_with("Could not connect to the Qdrant at qdrant.example.com:6334 (inline qdrant_url): the host name does not resolve"));
        assert!(unreachable_message(url, Some("lab"), "operation timed out")
            .contains("store 'lab' (qdrant.example.com:6334): no answer in time"));
        // An error we cannot classify is reported, never hidden.
        assert!(
            unreachable_message(url, Some("lab"), "collection quota exceeded")
                .ends_with("collection quota exceeded")
        );
    }
}
