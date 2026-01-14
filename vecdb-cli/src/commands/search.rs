use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use std::sync::Arc;
use crate::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;
use vecdb_common::output::OutputFormat;

pub async fn run(args: vecdb_core::tools::SearchArgs, config: &Config, profile_name: &str, format: OutputFormat) -> anyhow::Result<()> {
    let profile = config.resolve_profile(Some(profile_name), args.collection.as_deref())?;
    let show_progress = format == OutputFormat::Markdown && OUTPUT.is_interactive;
    
    if show_progress {
        println!("Using Profile: {} (Collection: {})", profile_name, profile.default_collection_name);
    }
    
    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let core = vecdb_core::Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &profile.embedding_model,
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(args.collection.as_deref()),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        config.smart_routing_keys.clone(),
        config.ingestion.path_rules.clone(),
        config.ingestion.max_concurrent_requests, 
        config.ingestion.gpu_batch_size,          
        file_detector.clone(),
        parser_factory.clone(),
    ).await?;
    
    let results = if args.smart {
        if show_progress {
            println!("Searching with smart routing in collection: {} for: {}", profile.default_collection_name, args.query);
        }
        core.search_smart(&profile.default_collection_name, &args.query, 10).await?
    } else {
        if show_progress {
            println!("Searching in collection: {} for: {}", profile.default_collection_name, args.query);
        }
        core.search(&profile.default_collection_name, &args.query, 10, None).await?
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
                    println!("\n--- Result {} (Score: {:.4}) ---", i + 1, result.score);
                    println!("{}", result.content.trim());
                }
            }
        }
    }
    
    Ok(())
}
