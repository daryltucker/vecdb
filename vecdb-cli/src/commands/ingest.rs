use clap::Args;
use std::path::PathBuf;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use std::sync::Arc;
use crate::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct IngestArgs {
    /// Path to the directory or file to ingest
    #[arg(index = 1, default_value = ".")]
    pub path: PathBuf,

    /// Collection to ingest into (created if missing)
    #[arg(long, short)]
    pub collection: Option<String>,

    /// Additional metadata in key=value format (can be specified multiple times)
    #[arg(long, short = 'm')]
    pub metadata: Vec<String>,

    /// Respect .gitignore files (skips ignored files)
    #[arg(long, default_value_t = false)]
    pub respect_gitignore: bool,

    /// Target chunk size (tokens for text, chars for default). Overrides config.
    #[arg(long)]
    pub chunk_size: Option<usize>,

    /// Chunk overlap (tokens for text, chars for default). Overrides config.
    #[arg(long, short = 'o')]
    pub overlap: Option<usize>,

    /// Extension whitelist (e.g. "rs", "md")
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub extensions: Option<Vec<String>>,

    /// Exclude glob patterns (e.g. "*.tmp", "target/")
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub excludes: Option<Vec<String>>,

    /// Dry run: List files that would be ingested without processing
    #[arg(long)]
    pub dry_run: bool,

    /// Max concurrent file processing tasks
    #[arg(long, short = 'P')]
    pub concurrency: Option<usize>,

    /// Max concurrent GPU embedding tasks (batch size)
    #[arg(long, short = 'G')]
    pub gpu_concurrency: Option<usize>,
}

pub async fn run(args: IngestArgs, config: &Config, profile_name: &str) -> anyhow::Result<()> {
    // Resolve profile with collection context
    let profile = config.resolve_profile(Some(profile_name), args.collection.as_deref())?;
    
    if OUTPUT.is_interactive && !args.dry_run {
        println!("Using Profile: {} (Collection: {})", profile_name, profile.default_collection_name);
    }
    
    // Check for stdin pipe
    if args.path.to_str() == Some("-") {
        return run_stdin(args, config, profile_name, &profile).await;
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
        config.ingestion.max_concurrent_requests, // Pass default concurrency
        config.ingestion.gpu_batch_size,          // Pass default GPU batch size
        file_detector.clone(),
        parser_factory.clone(),
    ).await?;

    // Parse metadata
    let mut metadata = std::collections::HashMap::new();
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            metadata.insert(key.to_string(), serde_json::Value::String(value.to_string()));
        }
    }

    if OUTPUT.is_interactive && !args.dry_run {
        println!("Ingesting content from: {:?} into collection: {}", args.path, profile.default_collection_name);
    }
    
    let resolved_chunk_size = config.resolve_chunk_size(args.collection.as_deref());
    let resolved_max_chunk_size = config.resolve_max_chunk_size(args.collection.as_deref());
    let resolved_overlap = config.resolve_chunk_overlap(args.collection.as_deref());
    
    let final_chunk_size = args.chunk_size.or(Some(resolved_chunk_size));
    let final_overlap = args.overlap.or(Some(resolved_overlap));
    let final_respect_gitignore = args.respect_gitignore || config.ingestion.respect_gitignore;
    
    tokio::select! {
             res = core.ingest(
                args.path.to_str().unwrap_or(""), 
                &profile.default_collection_name, 
                final_respect_gitignore, 
                final_chunk_size, 
                resolved_max_chunk_size, 
                final_overlap,
                args.extensions,
                args.excludes,
                args.dry_run,
                if metadata.is_empty() { None } else { Some(metadata) },

                args.concurrency, // Pass concurrency override
                args.gpu_concurrency, // Pass GPU concurrency override
                profile.quantization.clone(),
            ) => {
                res?;
                if OUTPUT.is_interactive && !args.dry_run {
                    println!("Ingestion complete.");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                // Flush stdout/stderr
                println!("\nCancelled by user.");
                return Ok(());
            }
        }
    
    Ok(())
}

async fn run_stdin(args: IngestArgs, config: &Config, _profile_name: &str, profile: &vecdb_core::config::Profile) -> anyhow::Result<()> {
    if OUTPUT.is_interactive {
        println!("Ingesting from stdin into collection: {}...", profile.default_collection_name);
    }
    
    let mut buffer = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)?;
    
    if buffer.trim().is_empty() {
        eprintln!("Warning: Empty input from stdin.");
        return Ok(());
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

    let mut metadata = std::collections::HashMap::new();
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            metadata.insert(key.to_string(), serde_json::Value::String(value.to_string()));
        }
    }
    metadata.entry("source".to_string()).or_insert(serde_json::Value::String("stdin".to_string()));
    
    let resolved_chunk_size = config.resolve_chunk_size(args.collection.as_deref());
    let resolved_max_chunk_size = config.resolve_max_chunk_size(args.collection.as_deref());
    let resolved_overlap = config.resolve_chunk_overlap(args.collection.as_deref());
    
    let final_chunk_size = args.chunk_size.or(Some(resolved_chunk_size));
    let final_overlap = args.overlap.or(Some(resolved_overlap));
    
    tokio::select! {
        res = core.ingest_content(&buffer, metadata, &profile.default_collection_name, final_chunk_size, resolved_max_chunk_size, final_overlap, profile.quantization.clone()) => {
            res?;
            println!("Ingestion complete.");
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nCancelled by user.");
            return Ok(());
        }
    }
    Ok(())
}
