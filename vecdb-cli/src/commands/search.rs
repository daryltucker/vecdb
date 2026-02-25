use crate::vecq_adapter::VecqParserFactory;
use std::sync::Arc;
use vecdb_common::output::OutputFormat;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use vecq::detection::HybridDetector;

pub async fn run(
    args: vecdb_core::tools::SearchArgs,
    config: &Config,
    profile_name: &str,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let profile = config.resolve_profile(Some(profile_name), args.collection.as_deref())?;
    let show_progress = format == OutputFormat::Markdown && OUTPUT.is_interactive;

    if show_progress {
        println!(
            "Using Profile: {} (Collection: {})",
            profile_name, profile.default_collection_name
        );
    }

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let core = vecdb_core::Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &config.resolve_embedding_model(&profile),
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(args.collection.as_deref()),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        config.smart_routing_keys.clone(),
        config.ingestion.path_rules.clone(),
        config.ingestion.max_concurrent_requests,
        config.resolve_gpu_batch_size(&profile, args.collection.as_deref()),
        profile.num_ctx,
        file_detector.clone(),
        parser_factory.clone(),
    )
    .await?;

    let results = if args.smart {
        if show_progress {
            println!(
                "Searching with smart routing in collection: {} for: {}",
                profile.default_collection_name, args.query
            );
        }
        core.search_smart(&profile.default_collection_name, &args.query, 10)
            .await?
    } else {
        if show_progress {
            println!(
                "Searching in collection: {} for: {}",
                profile.default_collection_name, args.query
            );
        }
        core.search(&profile.default_collection_name, &args.query, 10, None)
            .await?
    };

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string(&results)?);
        }
        _ => {
            if results.is_empty() {
                println!("No results found.");
            } else {
                for (i, result) in results.iter().enumerate() {
                    let path = result
                        .metadata
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let line_start = result.metadata.get("line_start").and_then(|v| v.as_u64());
                    let line_end = result.metadata.get("line_end").and_then(|v| v.as_u64());

                    let location = if let (Some(s), Some(e)) = (line_start, line_end) {
                        format!("{} [L{}-{}]", path, s, e)
                    } else {
                        path.to_string()
                    };

                    println!("\n--- Result {} (Score: {:.4}) | {} ---", i + 1, result.score, location);
                    println!("{}", result.content.trim());
                }
            }
        }
    }

    Ok(())
}
