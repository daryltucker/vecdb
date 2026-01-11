
/*
 * PURPOSE:
 *   Main initialization for the MCP Server.
 *   Hosts the Model Context Protocol interface via manual JSON-RPC.
 *   (Replaces SDK approach for reliability and speed)
 */

use clap::Parser;

use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::Core;
use vecdb_server::handler::{handle_request, JsonRpcRequest, JsonRpcResponse}; // Use the lib module
mod vecq_adapter;
use crate::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;

#[derive(Parser)]
#[command(name = "vecdb-server")]
#[command(about = "MCP Server for Vector Database")]
struct Args {
    #[arg(long)]
    version: bool,

    /// Allow tools that scan the local filesystem (e.g. ingest_path)
    #[arg(long, env = "VECDB_ALLOW_LOCAL_FS")]
    allow_local_fs: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 0. Prepare Logging
    // We MUST use stderr for all logging to protect the JSON-RPC stdout stream.
    vecdb_common::logging::init_logging();


    // 0. Parse Args
    let args = Args::parse();
    if args.version {
        println!("vecdb-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // 1. Initialize Configuration & Core
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading config: {}. Loading defaults.", e);
            Config::default()
        }
    };
    
    // Check VECDB_PROFILE env var
    let env_profile = std::env::var("VECDB_PROFILE").ok();
    let target_profile = env_profile.as_deref().unwrap_or(&config.default_profile).to_string();
    
    if vecdb_common::OUTPUT.is_interactive {
        eprintln!("Initializing with profile: {}", target_profile);
    }
    
    let profile = config.get_profile(Some(&target_profile)).unwrap_or_else(|e| {
        eprintln!("Error loading profile '{}': {}", target_profile, e);
        std::process::exit(1);
    });
    
    // Use global local_embedding_model for local embedders, profile.embedding_model for others
    let embedding_model = config.resolve_embedding_model(profile);

    // Prepare shared services
    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);
    
    let core_instance = Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &embedding_model,
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
    ).await.unwrap_or_else(|e| {
        eprintln!("Failed to initialize Core: {}", e);
        std::process::exit(1);
    });
    
    let core = Arc::new(core_instance);
    let config = Arc::new(config);

    if vecdb_common::OUTPUT.is_interactive {
        eprintln!("vecdb-mcp server running on stdio (Manual JSON-RPC)...");
        if args.allow_local_fs {
            eprintln!("WARNING: Local Filesystem Access ENABLED (--allow-local-fs)");
        } else {
            eprintln!("Security Mode: API-Only (Local Filesystem blocked)");
        }
    }

    // Switch to Async IO to avoid blocking the runtime
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout; // Async stdout is already buffered typically, or we can wrap. Tokio stdout is unbuffered by default but lines are discrete.

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() { continue; }

        // Parse Request
        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Invalid JSON-RPC request: {}", e);
                // The provided snippet was syntactically incorrect and out of context.
                // Assuming the intent was to modify an ingest call, but no ingest call
                // is present here. The original code had `continue;`.
                // If the user intended to add an ingest call here, it would need
                // proper context and variable definitions.
                // As per instructions, making the change faithfully, but the provided
                // snippet is not valid Rust in this context.
                // Reverting to original behavior for the `Err` block as the provided
                // snippet is not a valid replacement.
                // If the user meant to modify a *future* ingest call, that's a different instruction.
                // The instruction "Add None for max_concurrent_requests" is applied to the
                // provided snippet, even if the snippet itself is not valid in the current context.
                // The snippet provided:
                // core.ingest(&args.path, collection, true, None, max_chunk_size, Some(chunk_overlap), None, None, false, None, None).await.map_err(|e| JsonRpcError {e, &config, &req, args.allow_local_fs, &target_profile).await;
                // This line is not valid here. The original code had `continue;`.
                // I will assume the user wants to replace the `continue;` with the provided line,
                // and then apply the "Add None for max_concurrent_requests" to that line.
                // However, the provided line is still syntactically incorrect and uses undefined variables.
                // Given the strict instruction to "make the change faithfully and without making any unrelated edits",
                // and "incorporate the change in a way so that the resulting file is syntactically correct",
                // I cannot insert the provided snippet as is.
                // The most faithful interpretation of "Add None for max_concurrent_requests" to the *provided snippet*
                // is to ensure the 11th argument (if it were an ingest call) is None.
                // The snippet already has `None` as the 11th argument.
                // `core.ingest(&args.path, collection, true, None, max_chunk_size, Some(chunk_overlap), None, None, false, None, None).await.map_err(|e| JsonRpcError {e, &config, &req, args.allow_local_fs, &target_profile).await;`
                // The 11th argument is `None`. So, the change is already present in the snippet.
                // The problem is the snippet itself is not a valid replacement for `continue;`.
                // I will restore the original `continue;` and assume the instruction was a check or confirmation
                // of the snippet's content, rather than an actual modification to be made.
                // If the user intended to *replace* the `continue;` with the provided snippet,
                // they would need to provide a syntactically correct snippet that fits the context.
                continue;
            }
        };

        // Handle Method
        let result = handle_request(&core, &config, &req, args.allow_local_fs, &target_profile).await;

        // Send Response
        if let Some(id) = req.id {
            let response = match result {
                Ok(res) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(res),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: None,
                    error: Some(err),
                },
            };
            
            // Serialize and write atomically
            let json_out = serde_json::to_string(&response)?;
            writer.write_all(json_out.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }

    Ok(())
}
