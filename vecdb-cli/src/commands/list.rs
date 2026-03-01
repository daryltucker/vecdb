use crate::vecq_adapter::VecqParserFactory;
use std::io::Write;
use std::sync::Arc;
use vecdb_common::output::OutputFormat;
use vecdb_core::config::Config;
use vecdb_core::config::QuantizationType;
use vecq::detection::HybridDetector;

pub async fn run(config: &Config, profile_name: Option<&str>, format: OutputFormat) -> anyhow::Result<()> {
    // Resolve profile (generic collection)
    let profile = config.resolve_profile(profile_name, None)?;
    
    // Performance: Avoid connecting to Ollama/loading local model just to list collections
    std::env::set_var("VECDB_SKIP_PROBE", "true");

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let core = vecdb_core::Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &config.resolve_embedding_model(&profile), // Fixed: pass reference
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(None),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        config.smart_routing_keys.clone(),
        config.ingestion.path_rules.clone(),
        config.ingestion.max_concurrent_requests,
        config.resolve_gpu_batch_size(&profile, None), // No collection context known
        profile.num_ctx,
        file_detector.clone(),
        parser_factory.clone(),
    )
    .await?;

    let collections = core.list_collections().await?;

    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    match format {
        OutputFormat::Json => {
            serde_json::to_writer(&mut out, &collections)?;
            writeln!(out)?;
        }
        _ => {
            if collections.is_empty() {
                writeln!(out, "No collections found.")?;
            } else {
                writeln!(
                    out,
                    "{:<20} | {:<15} | {:<10} | {:<10}",
                    "Name", "Vectors", "Dim", "Quant"
                )?;
                writeln!(out, "{:-<20}-+-{:-<15}-+-{:-<10}-+-{:-<10}", "", "", "", "")?;
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
                        "{:<20} | {:<15} | {:<10} | {:<10}",
                        c.name, count, dim, quant
                    )?;
                    if size_gb > 4.0 {
                        if matches!(
                            c.quantization,
                            Some(QuantizationType::Scalar) | Some(QuantizationType::Binary)
                        ) {
                            writeln!(out, "  ^-- NOTE: Approx {:.2} GB RAM (Optimized).", size_gb)?;
                        } else {
                            writeln!(
                                out,
                                "  ^-- WARNING: Approx {:.2} GB RAM. Consider 'vecdb optimize {}'",
                                size_gb, c.name
                            )?;
                        }
                    }
                }
            }
        }
    }
    out.flush()?;
    Ok(())
}
