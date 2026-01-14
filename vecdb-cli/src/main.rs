//! DOCS: docs/CLI.md
//! COMPLIANCE: tests/tier2_cli_compliance.py
/*
 * PURPOSE:
 *   Main entry point for vecdb-cli.
 *   Parses arguments and dispatches to subcommands.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - CLI structure (init, ingest, search, man)
 *   - Config file with profiles (User Prompt)
 *   - Default profile capability
 *
 * IMPLEMENTATION RULES:
 *   1. Use `clap` derive pattern
 *      Rationale: Type-safe argument parsing.
 *   2. Load Config early
 *      Rationale: Fail fast if config is corrupt (unless init).
 */

use clap::{Parser, Subcommand, CommandFactory};
use clap_complete::{generate, Shell};
use std::path::PathBuf;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
mod commands;
mod vecq_adapter; // Add the adapter module

// SAFETY: Jemalloc is configured as the global allocator for Linux targets to reduce fragmentation
// in long-running async server workloads (ingestion/search).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::sync::Arc;
use vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;


/// Helper: Resolve embedding model, using global local_embedding_model for local profiles

#[derive(clap::ValueEnum, Clone, Debug)]
enum QuantizationArg {
    Scalar,
    Binary,
    None,
}

impl From<QuantizationArg> for vecdb_core::config::QuantizationType {
    fn from(val: QuantizationArg) -> Self {
        match val {
            QuantizationArg::Scalar => vecdb_core::config::QuantizationType::Scalar,
            QuantizationArg::Binary => vecdb_core::config::QuantizationType::Binary,
            QuantizationArg::None => vecdb_core::config::QuantizationType::None,
        }
    }
}


#[derive(Parser, Debug)]
#[command(name = "vecdb")]
#[command(about = "Vector Database Project CLI", long_about = None)]
#[command(after_help = "See `vecdb man --agent` for Agent Interface documentation.")]
struct Cli {
    /// Profile to use from config.toml
    #[arg(long, global = true)]
    profile: Option<String>,

    /// Force JSON output
    #[arg(long, short = 'j', global = true)]
    json: bool,

    /// Force Markdown/Text output
    #[arg(long, short = 'm', global = true)]
    markdown: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize configuration
    Init,

    /// Recursively ingest documents from a path into a collection.
    /// Supports .gitignore (optional) and .vectorignore (always), custom chunking, and metadata tagging.
        Ingest {
            /// Path to the directory or file to ingest
            #[arg(index = 1, default_value = ".")]
            path: PathBuf,

            /// Collection to ingest into (created if missing)
            #[arg(long, short)]
            collection: Option<String>,

            /// Additional metadata in key=value format (can be specified multiple times)
            #[arg(long, short = 'm')]
            metadata: Vec<String>,

            /// Respect .gitignore files (skips ignored files)
            #[arg(long, default_value_t = false)]
            respect_gitignore: bool,

            /// Target chunk size (tokens for text, chars for default). Overrides config.
            #[arg(long)]
            chunk_size: Option<usize>,

            /// Chunk overlap (tokens for text, chars for default). Overrides config.
            #[arg(long, short = 'o')]
            overlap: Option<usize>,

            /// Extension whitelist (e.g. "rs", "md")
            #[arg(long, value_delimiter = ',', num_args = 1..)]
            extensions: Option<Vec<String>>,

            /// Exclude glob patterns (e.g. "*.tmp", "target/")
            #[arg(long, value_delimiter = ',', num_args = 1..)]
            excludes: Option<Vec<String>>,

            /// Dry run: List files that would be ingested without processing
            #[arg(long)]
            dry_run: bool,

            /// Max concurrent file processing tasks
            #[arg(long, short = 'P')]
            concurrency: Option<usize>,

            /// Max concurrent GPU embedding tasks (batch size)
            #[arg(long, short = 'G')]
            gpu_concurrency: Option<usize>,
        },

        /// Search the index
        Search(vecdb_core::tools::SearchArgs),

        /// List available collections
        List,

        /// Show system status and connectivity
        Status(commands::status::StatusArgs),

        /// Delete a collection
        Delete(commands::delete::DeleteArgs),

        /// Manage Collection Snapshots (Create, List, Download, Restore)
        Snapshot {
            #[arg(short, long)]
            create: bool,

            #[arg(short, long)]
            list: bool,

            #[arg(short, long)]
            download: Option<String>, // Snapshot name

            #[arg(long)]
            restore: Option<String>, // File path

            #[arg(short = 'C', long)]
            collection: Option<String>, // Optional override
        },

        /// Display manual
        Man(commands::man::ManArgs),

        /// Manage config settings
        Config {
            #[command(subcommand)]
            command: ConfigCommands,
        },
        
        /// Optimize a collection (apply quantization)
        Optimize {
            /// Collection to optimize
            #[arg(index = 1)]
            collection: String,
        },

        /// Time Travel / History Operations
        History {
            #[command(subcommand)]
            command: HistoryCommands,
        },
        /// Generate shell completions
        Completions {
            /// Shell to generate completions for
            #[arg(value_enum)]
            shell: Shell,
        },
    }

    #[derive(Subcommand, Debug)]
    enum HistoryCommands {
        /// Ingest a specific version of a repository
        Ingest {
            /// Git reference (SHA, tag, branch)
            #[arg(long, short = 'r')]
            git_ref: String,

            /// Repository path (defaults to current dir)
            #[arg(default_value = ".")]
            path: String,

            /// Collection
            #[arg(long, short, default_value = "docs")]
            collection: String,
        },
    }

    #[derive(Subcommand, Debug)]
    enum ConfigCommands {
        /// Set quantization for a collection
        SetQuantization {
             /// Collection name
             #[arg(index=1)]
             collection: String,
             /// Quantization type (scalar, binary, none)
             #[arg(value_enum, index=2)]
             r#type: QuantizationArg,
        },
    }


    #[tokio::main]
    async fn main() -> anyhow::Result<()> {
        // Initialize logging (clean production default)
        vecdb_common::logging::init_logging(); 
        
        let result = try_main().await;
        
        // Handle SIGPIPE (Broken Pipe) gracefully
        // The Report says: "Wrap the main logic in a handler that catches std::io::Error. If error.kind() == std::io::ErrorKind::BrokenPipe, exit(0)"
        if let Err(err) = result {
             // Try to downcast to io::Error
             if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
                 if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                     // Silently exit with success (standard Unix behavior for pipe closed)
                     std::process::exit(0);
                 }
             }
             // For now, clap handles help/error printing usually, but if we bubble up:
             // We print and return Err (which prints again?). 
             // anyhow::Result in main prints with Debug.
             return Err(err);
        }
        Ok(())
    }

    async fn try_main() -> anyhow::Result<()> {
        // Build Version String
        let app_version = env!("CARGO_PKG_VERSION");
        let ort_version = vecdb_core::get_ort_version();
        let long_version = format!(
            "vecdb v{}\nONNX v{}", 
            app_version, 
            ort_version
        );

        // We manually build the command to inject the version
        let long_version_static: &'static str = Box::leak(long_version.into_boxed_str());
        let cmd = Cli::command().version(long_version_static);
        
        // Parse using the modified command definition
        let matches = cmd.get_matches();
        
        // Convert matches back to Cli struct
        use clap::FromArgMatches;
        let cli = Cli::from_arg_matches(&matches)?;
        
        // Safety Check for Init:
        // We must check existence BEFORE Config::load() because load() auto-creates the file.
        if let Commands::Init = cli.command {
                let path = Config::get_path()?;
                if path.exists() {
                    // If it exists, we ABORT to prevent accidental overwrite or confusion.
                    // The user must manually handle the file if they want to reset.
                    eprintln!("❌ Config file already exists at: {:?}", path);
                    eprintln!("   Aborting `init` to prevent accidental overwrite.");
                    eprintln!("   To reset: backup/delete the file and run `vecdb init` again.");
                    std::process::exit(1);
                }
            }

            // Load Configuration
            let mut config = Config::load()?; // Changed to `mut config`
            let base_profile_name = cli.profile.as_deref().unwrap_or(&config.default_profile);
            
            // Prepare shared services for injection
            // These are effectively singletons for the CLI run
            let file_detector = Arc::new(HybridDetector::new());
            let parser_factory = Arc::new(VecqParserFactory);

            // Profile resolution is deferred to the specific commands which know the collection context

            let format = resolve_format_flags(cli.json, cli.markdown);

            match cli.command {
                Commands::Completions { shell } => {
                    let mut cmd = Cli::command();
                    generate(shell, &mut cmd, "vecdb", &mut std::io::stdout());
                    return Ok(());
                }
                Commands::Man(args) => commands::man::run(args)?,
                Commands::Status(args) => {
                     commands::status::run(args, &config, base_profile_name, format).await?;
                }
                Commands::Delete(args) => {
                    let profile = config.resolve_profile(Some(base_profile_name), args.collection.as_deref())?;
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
                        config.ingestion.max_concurrent_requests, // Pass default concurrency
                        config.ingestion.gpu_batch_size,          // Pass default GPU batch size
                        file_detector.clone(),
                        parser_factory.clone(),
                    ).await?;
                    commands::delete::run(&core, args).await?;
                }
                Commands::Init => {
                    let path = Config::get_path()?; // Should exist now since load() created it
                    println!("✅ Initialized new configuration at: {:?}", path);
                    println!("   Default Profile: {}", config.default_profile);
                    println!("   Edit this file to configure your profiles and keys.");
                }
                Commands::Ingest { path, collection, metadata: meta_args, respect_gitignore, chunk_size, overlap, extensions, excludes, dry_run, concurrency, gpu_concurrency } => {
                    // Resolve profile with collection context
                    let profile = config.resolve_profile(Some(base_profile_name), collection.as_deref())?;
                    if OUTPUT.is_interactive {
                        println!("Using Profile: {} (Collection: {})", base_profile_name, profile.default_collection_name);
                    }
                    
                    let core = vecdb_core::Core::new(
                        &profile.qdrant_url,
                        &profile.ollama_url,
                        &profile.embedding_model,
                        profile.accept_invalid_certs,
                        &profile.embedder_type,
                        Some(config.fastembed_cache_path.clone()),
                        config.resolve_local_use_gpu(collection.as_deref()),
                        profile.qdrant_api_key.clone(),
                        profile.ollama_api_key.clone(),
                        config.smart_routing_keys.clone(),
                        config.ingestion.path_rules.clone(),
                        config.ingestion.max_concurrent_requests, // Pass default concurrency
                        config.ingestion.gpu_batch_size,          // Pass default GPU batch size
                        file_detector.clone(),
                        parser_factory.clone(),
                    ).await?;

                    let mut metadata = std::collections::HashMap::new();
                    for item in meta_args {
                        if let Some((key, value)) = item.split_once('=') {
                            metadata.insert(key.to_string(), serde_json::Value::String(value.to_string()));
                        }
                    }

                    if path.to_str() == Some("-") {
                        // Read from stdin (Pipe Mode)
                        if OUTPUT.is_interactive {
                            println!("Ingesting from stdin into collection: {}...", profile.default_collection_name);
                        }
                        let mut buffer = String::new();
                        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)?;
                        
                        if buffer.trim().is_empty() {
                            eprintln!("Warning: Empty input from stdin.");
                        } else {
                            // Add source metadata if not provided
                            metadata.entry("source".to_string()).or_insert(serde_json::Value::String("stdin".to_string()));
                            
                            let resolved_chunk_size = config.resolve_chunk_size(collection.as_deref());
                            let resolved_max_chunk_size = config.resolve_max_chunk_size(collection.as_deref());
                            let resolved_overlap = config.resolve_chunk_overlap(collection.as_deref());
                            
                            let final_chunk_size = chunk_size.or(Some(resolved_chunk_size));
                            let final_overlap = overlap.or(Some(resolved_overlap));
                            
                            // Note: Use resolved profile's collection name
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
                        }
                    } else {
                        // File/Dir Mode
                        // Note: ingest() in Core doesn't currently take extra metadata, 
                        // it should be updated if we want to support it for file/dir too.
                        if OUTPUT.is_interactive && !dry_run {
                            println!("Ingesting content from: {:?} into collection: {}", path, profile.default_collection_name);
                        }
                        let resolved_chunk_size = config.resolve_chunk_size(collection.as_deref());
                        let resolved_max_chunk_size = config.resolve_max_chunk_size(collection.as_deref());
                        let resolved_overlap = config.resolve_chunk_overlap(collection.as_deref());
                        
                        let final_chunk_size = chunk_size.or(Some(resolved_chunk_size));
                        let final_overlap = overlap.or(Some(resolved_overlap));
                        let final_respect_gitignore = respect_gitignore || config.ingestion.respect_gitignore;
                        
                        tokio::select! {
                             res = core.ingest(
                                path.to_str().unwrap_or(""), 
                                &profile.default_collection_name, 
                                final_respect_gitignore, 
                                final_chunk_size, 
                                resolved_max_chunk_size, 
                                final_overlap,
                                extensions,
                                excludes,
                                dry_run,
                                if metadata.is_empty() { None } else { Some(metadata) },

                                concurrency, // Pass concurrency override
                                gpu_concurrency, // Pass GPU concurrency override
                                profile.quantization.clone(),
                            ) => {
                                res?;
                                if OUTPUT.is_interactive && !dry_run {
                                    println!("Ingestion complete.");
                                }
                            }
                            _ = tokio::signal::ctrl_c() => {
                                // Flush stdout/stderr
                                println!("\nCancelled by user.");
                                return Ok(());
                            }
                        }
                    }
                }
            Commands::Search(args) => {
                let profile = config.resolve_profile(Some(base_profile_name), args.collection.as_deref())?;
                let show_progress = format == vecdb_common::output::OutputFormat::Markdown && OUTPUT.is_interactive;
                
                if show_progress {
                    println!("Using Profile: {} (Collection: {})", base_profile_name, profile.default_collection_name);
                }
                
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
                    vecdb_common::output::OutputFormat::Json => {
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
            }
            Commands::List => {
                let profile = config.resolve_profile(Some(base_profile_name), None)?;
                
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
                    config.ingestion.gpu_batch_size,          
                    file_detector.clone(),
                    parser_factory.clone(),
                ).await?;
                
                let collections = core.list_collections().await?;
                
                // Buffered I/O for performance and atomic writes
                use std::io::Write;
                let stdout = std::io::stdout();
                let mut out = std::io::BufWriter::new(stdout.lock());

                match format {
                    vecdb_common::output::OutputFormat::Json => {
                        serde_json::to_writer(&mut out, &collections)?;
                        writeln!(out)?; // Trailing newline
                    }
                    _ => {
                        if collections.is_empty() {
                            writeln!(out, "No collections found.")?;
                        } else {
                            writeln!(out, "{:<20} | {:<15} | {:<10} | {:<10}", "Name", "Vectors", "Dim", "Quant")?;
                            writeln!(out, "{:-<20}-+-{:-<15}-+-{:-<10}-+-{:-<10}", "", "", "", "")?;
                            for c in collections {
                                let count_val = c.vector_count.unwrap_or(0);
                                let dim_val = c.vector_size.unwrap_or(0);
                                let (bytes_per_dim, overhead_mult) = match c.quantization {
                                    Some(vecdb_core::config::QuantizationType::Scalar) => (1.0, 1.2),
                                    Some(vecdb_core::config::QuantizationType::Binary) => (0.125, 1.2),
                                    _ => (4.0, 1.2),
                                };
        
                                let total_bytes = (count_val as f64 * dim_val as f64 * bytes_per_dim) * overhead_mult;
                                let size_gb = total_bytes / (1024.0 * 1024.0 * 1024.0);
                                
                                let count = c.vector_count.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
                                let dim = c.vector_size.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
                                let quant = match c.quantization {
                                    Some(vecdb_core::config::QuantizationType::Scalar) => "Scalar",
                                    Some(vecdb_core::config::QuantizationType::Binary) => "Binary",
                                    _ => "None", 
                                };
                                
                                writeln!(out, "{:<20} | {:<15} | {:<10} | {:<10}", c.name, count, dim, quant)?;
                                if size_gb > 4.0 {
                                      if matches!(c.quantization, Some(vecdb_core::config::QuantizationType::Scalar) | Some(vecdb_core::config::QuantizationType::Binary)) {
                                         writeln!(out, "  ^-- NOTE: Approx {:.2} GB RAM (Optimized).", size_gb)?;
                                     } else {
                                         writeln!(out, "  ^-- WARNING: Approx {:.2} GB RAM. Consider 'vecdb optimize {}'", size_gb, c.name)?;
                                     }
                                }
                            }
                        }
                    }
                }
                out.flush()?;
            }
            Commands::History { command } => {
                match command {
                    HistoryCommands::Ingest { git_ref, path, collection } => {
                         let profile = config.resolve_profile(Some(base_profile_name), Some(&collection))?;
                         
                         let core = vecdb_core::Core::new(
                            &profile.qdrant_url,
                            &profile.ollama_url,
                            &config.resolve_embedding_model(&profile),
                            profile.accept_invalid_certs,
                            &profile.embedder_type,
                            Some(config.fastembed_cache_path.clone()),
                            config.resolve_local_use_gpu(Some(&collection)),
                            profile.qdrant_api_key.clone(),
                            profile.ollama_api_key.clone(),
                            config.smart_routing_keys.clone(),
                            config.ingestion.path_rules.clone(),
                            config.ingestion.max_concurrent_requests, 
                            config.ingestion.gpu_batch_size,          // Pass default GPU batch size
                            file_detector.clone(),
                            parser_factory.clone(),
                        ).await?;

                         if OUTPUT.is_interactive {
                             println!("Time Traveling to: {} @ {} (Collection: {})", path, git_ref, profile.default_collection_name);
                         }
                         core.ingest_history(&path, &git_ref, &profile.default_collection_name, 512, profile.quantization.clone()).await?;
                    }
                }
            }
                Commands::Snapshot { create, list, download, restore, collection } => {
                    let profile = config.resolve_profile(Some(base_profile_name), collection.as_deref())?;
                    let collection_name = collection.as_deref().unwrap_or(&profile.default_collection_name);
                    
                    let manager = vecdb_core::snapshot::SnapshotManager::new(&profile.qdrant_url)?;

                    if create {
                        if OUTPUT.is_interactive { println!("Creating snapshot for collection '{}'...", collection_name); }
                        let name = manager.create(collection_name).await?;
                        println!("Snapshot created: {}", name);
                    } else if list {
                        let snapshots = manager.list(collection_name).await?;
                        if snapshots.is_empty() {
                             println!("No snapshots found for collection '{}'.", collection_name);
                        } else {
                            println!("Snapshots for '{}':", collection_name);
                            for s in snapshots {
                                println!("- {}", s);
                            }
                        }
                    } else if let Some(snap_name) = download {
                        let output_path = std::path::Path::new(&snap_name);
                        if OUTPUT.is_interactive { println!("Downloading snapshot '{}'...", snap_name); }
                        manager.download(collection_name, &snap_name, output_path).await?;
                        println!("Downloaded to: {:?}", output_path);
                    } else if let Some(file_path) = restore {
                         if OUTPUT.is_interactive { println!("Restoring snapshot from {:?} to collection '{}'...", file_path, collection_name); }
                        manager.restore(collection_name, std::path::Path::new(&file_path)).await?;
                        println!("Snapshot restored successfully.");
                    } else {
                        println!("Please specify an action: --create, --list, --download <NAME>, or --restore <PATH>");
                    }
                }

                Commands::Config { command } => {
                    match command {
                        ConfigCommands::SetQuantization { collection, r#type } => {
                            let q_type: vecdb_core::config::QuantizationType = r#type.into();
                            
                            let c_config = config.collections.entry(collection.clone()).or_insert(vecdb_core::config::CollectionConfig {
                                name: collection.clone(),
                                description: None,
                                embedder_type: None,
                                embedding_model: None,
                                ollama_url: None,
                                chunk_size: None,
                                chunk_overlap: None,
                                max_chunk_size: None,
                                use_gpu: None,
                                qdrant_api_key: None,
                                ollama_api_key: None,
                                quantization: None,
                            });
                            
                            c_config.quantization = Some(q_type.clone());
                            config.save()?;
                            println!("Updated quantization for collection '{}' to {:?}", collection, q_type);
                        }
                    }
                }
                Commands::Optimize { collection } => {
                    let profile = config.resolve_profile(Some(base_profile_name), Some(&collection))?;
                    let q_type = profile.quantization.unwrap_or(vecdb_core::config::QuantizationType::Scalar);

                    if OUTPUT.is_interactive {
                        println!("Optimizing collection '{}' with strategy: {:?}", collection, q_type);
                    }

                    let core = vecdb_core::Core::new(
                        &profile.qdrant_url,
                        &profile.ollama_url,
                        &profile.embedding_model,
                        profile.accept_invalid_certs,
                        &profile.embedder_type,
                        Some(config.fastembed_cache_path.clone()),
                        config.resolve_local_use_gpu(Some(&collection)),
                        profile.qdrant_api_key.clone(),
                        profile.ollama_api_key.clone(),
                        config.smart_routing_keys.clone(),
                        config.ingestion.path_rules.clone(),
                        config.ingestion.max_concurrent_requests, 
                        config.ingestion.gpu_batch_size,          
                        file_detector.clone(),
                        parser_factory.clone(),
                    ).await?;

                    core.optimize_collection(&collection, q_type).await?;
                    println!("Optimization triggered. Check Qdrant logs for background progress.");
                }
            }
        Ok(())
    }

fn resolve_format_flags(json: bool, markdown: bool) -> vecdb_common::output::OutputFormat {
    if json {
         vecdb_common::output::OutputFormat::Json
    } else if markdown {
         vecdb_common::output::OutputFormat::Markdown
    } else {
         // Use detected default
         vecdb_common::output::OutputContext::detect().resolve_format()
    }
}
