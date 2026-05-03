use crate::vecq_adapter::VecqParserFactory;
use clap::Args;
use std::path::PathBuf;
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use vecdb_core::vecdbrc::VecdbRc;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct IngestArgs {
    /// Path(s) to files or directories to ingest. Use `-` for stdin.
    /// Glob expansion happens in your shell (bash/zsh) — vecdb receives
    /// the expanded paths. You can pass multiple paths: `ingest a.md b.md`
    #[arg(default_value = ".", num_args = 1..)]
    pub paths: Vec<PathBuf>,

    /// Collection to ingest into (created if missing)
    #[arg(long, short)]
    pub collection: Option<String>,

    /// Additional metadata in key=value format (can be specified multiple times)
    #[arg(long, short = 'm')]
    pub metadata: Vec<String>,

    /// Respect .gitignore files (skips ignored files)
    #[arg(long, default_value_t = false)]
    pub respect_gitignore: bool,

    /// Ignore .vectorignore files (skip reading .vectorignore during file walking)
    #[arg(long, default_value_t = false)]
    pub ignore_vectorignore: bool,

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

pub async fn run(args: IngestArgs, config: &Config, profile_name: Option<&str>) -> anyhow::Result<()> {
    // Resolve profile with collection context
    let profile = config.resolve_profile(profile_name, args.collection.as_deref())?;
    let display_profile = &profile.resolved_profile_name;

    let collection = profile.default_collection_name.as_deref()
        .ok_or_else(|| anyhow::anyhow!(
            "No collection specified. Use -c <name>, or point a collection to profile \"{}\" via `profile = \"{}\"` in config.",
            display_profile, display_profile
        ))?;

    if OUTPUT.is_interactive && !args.dry_run {
        println!(
            "Using Profile: {} (Collection: {})",
            display_profile, collection
        );
    }

    // Check for stdin pipe: any path value of "-" means read from stdin.
    // When multiple paths are given, only one "-" is expected alone.
    if args.paths.iter().any(|p| p.to_str() == Some("-")) {
        return run_stdin(args, config, display_profile, &profile, collection).await;
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
        config.resolve_gpu_batch_size(&profile, args.collection.as_deref()), // Smart dynamic sizing
        profile.num_ctx, // Propagate LLM Context limit
        file_detector.clone(),
        parser_factory.clone(),
    )
    .await?;

    // Parse metadata
    let mut metadata = std::collections::HashMap::new();
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            metadata.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    let resolved_chunk_size = config.resolve_chunk_size(args.collection.as_deref());
    let resolved_max_chunk_size = config.resolve_max_chunk_size(&profile, args.collection.as_deref());
    let resolved_overlap = config.resolve_chunk_overlap(&profile, args.collection.as_deref());

    let final_chunk_size = args.chunk_size.or(Some(resolved_chunk_size));
    let final_overlap = args.overlap.or(Some(resolved_overlap));
    let final_respect_gitignore = args.respect_gitignore || config.ingestion.respect_gitignore;

    // ── Multi-path batching ──────────────────────────────────────────
    // When multiple paths are given (glob expansion, CLI listing), we find
    // their common ancestor and pass the specific files as a file_allowlist.
    // This way ONE ingest pipeline discovers + parses + embeds all files,
    // showing proper progress (N/N) and capturing topographic metadata.
    let (ingest_path, file_allowlist, project_root_display) = if args.paths.len() <= 1 {
        // Single path: pass as-is
        let single = args.paths.first()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string();
        let single_display = single.clone();
        (single, None, single_display)
    } else {
        // Multiple paths: compute common ancestor
        let resolved: Vec<PathBuf> = args.paths.iter()
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .collect();

        let common = common_ancestor(&resolved)
            .unwrap_or_else(|| std::path::Path::new(".").to_path_buf());

        let allowlist: Vec<String> = resolved.iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let display = common.to_string_lossy().to_string();
        if OUTPUT.is_interactive {
            let n = allowlist.len();
            println!("Batch ingest: {} files under {}", n, display);
        }
        let display_for_meta = display.clone();
        (display, Some(allowlist), display_for_meta)
    };

    // .vecdbrc discovery from the first input path
    let rc_result = VecdbRc::discover(std::path::Path::new(&ingest_path))?;
    let (rc_path, rc) = match rc_result {
        Some((rc_path, rc)) => {
            if OUTPUT.is_interactive {
                println!("Found .vecdbrc at {} with {} route(s). Enabling per-file routing.",
                    rc_path.display(), rc.routes.len());
            }
            (Some(rc_path), Some(rc))
        }
        None => (None, None),
    };

    let vecdbrc_root = rc_path.as_ref()
        .and_then(|p| VecdbRc::project_root(p))
        .unwrap_or_else(|| std::path::Path::new(&ingest_path))
        .to_path_buf();

    // Build IngestionOptions with optional file_allowlist
    let opts = vecdb_core::ingestion::IngestionOptions {
        path: ingest_path.clone(),
        collection: collection.to_string(),
        file_allowlist,
        project_root: Some(project_root_display),
        vecdbrc_routes: rc.as_ref().map(|r| r.routes.clone()),
        vecdbrc_root: Some(vecdbrc_root),
        chunk_size: final_chunk_size.unwrap_or(vecdb_core::config::DEFAULT_CHUNK_SIZE),
        max_chunk_size: resolved_max_chunk_size,
        chunk_overlap: final_overlap.unwrap_or(50),
        respect_gitignore: final_respect_gitignore,
        ignore_vectorignore: args.ignore_vectorignore,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
        extensions: args.extensions.clone(),
        excludes: args.excludes.clone(),
        dry_run: args.dry_run,
        metadata: if metadata.is_empty() { None } else { Some(metadata.clone()) },
        path_rules: Vec::new(),
        max_concurrent_requests: args.concurrency.unwrap_or(4),
        gpu_batch_size: args.gpu_concurrency.unwrap_or(2),
        quantization: profile.quantization.clone(),
    };

    tokio::select! {
        res = core.ingest_with_options(opts, None) => {
            res?;
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nCancelled by user.");
            std::process::exit(0);
        }
    }

    if OUTPUT.is_interactive && !args.dry_run {
        println!("Ingestion complete.");
    }

    Ok(())
}

async fn run_stdin(
    args: IngestArgs,
    config: &Config,
    _profile_name: &str,
    profile: &vecdb_core::config::Profile,
    collection: &str,
) -> anyhow::Result<()> {
    if OUTPUT.is_interactive {
        println!(
            "Ingesting from stdin into collection: {}...",
            collection
        );
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
        &config.resolve_embedding_model(profile),
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(args.collection.as_deref()),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        config.smart_routing_keys.clone(),
        config.ingestion.path_rules.clone(),
        config.ingestion.max_concurrent_requests,
        config.resolve_gpu_batch_size(profile, args.collection.as_deref()),
        profile.num_ctx,
        file_detector.clone(),
        parser_factory.clone(),
    )
    .await?;

    let mut metadata = std::collections::HashMap::new();
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            metadata.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
    metadata
        .entry("source".to_string())
        .or_insert(serde_json::Value::String("stdin".to_string()));

    let resolved_chunk_size = config.resolve_chunk_size(args.collection.as_deref());
    let resolved_max_chunk_size = config.resolve_max_chunk_size(profile, args.collection.as_deref());
    let resolved_overlap = config.resolve_chunk_overlap(profile, args.collection.as_deref());

    let final_chunk_size = args.chunk_size.or(Some(resolved_chunk_size));
    let final_overlap = args.overlap.or(Some(resolved_overlap));

    tokio::select! {
        res = core.ingest_content(&buffer, metadata, collection, final_chunk_size, resolved_max_chunk_size, final_overlap, profile.quantization.clone(), None) => {
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

/// Find the common ancestor directory of a set of paths.
/// Returns the longest shared prefix path component.
fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        return paths[0].parent().map(|p| p.to_path_buf());
    }

    // Collect all ancestors of the first path
    let first = &paths[0];
    let ancestors: Vec<&std::path::Path> = first.ancestors().collect();

    // Find the longest ancestor shared by all paths
    for ancestor in ancestors {
        if paths.iter().all(|p| p.starts_with(ancestor)) {
            return Some(ancestor.to_path_buf());
        }
    }

    // Fall back to current directory
    Some(std::path::Path::new(".").to_path_buf())
}
