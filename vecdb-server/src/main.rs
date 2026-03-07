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
use vecdb_server::core_registry::{CoreFactory, CoreKey, CoreRegistry};
use vecdb_server::rpc::{handle_request, types::{JsonRpcRequest, JsonRpcResponse}};
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

    /// Run in legacy stdio mode (MCP default)
    #[arg(long)]
    stdio: bool,

    /// Port for HTTP server (default: 3000)
    #[arg(long, default_value = "3000")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install aws-lc-rs as the TLS crypto provider before any connections.
    // Required because fastembed (reqwest 0.12) and vecdb-core (reqwest 0.13) each
    // pull in a different rustls backend (ring vs aws-lc-rs), leaving rustls unable
    // to auto-select one. Must run before tokio or reqwest initialize TLS.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

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
    let target_profile = env_profile
        .as_deref()
        .unwrap_or(&config.default_profile)
        .to_string();

    if vecdb_common::OUTPUT.is_interactive {
        eprintln!("Initializing with profile: {}", target_profile);
    }

    let profile = config
        .get_profile(Some(&target_profile))
        .unwrap_or_else(|e| {
            eprintln!("Error loading profile '{}': {}", target_profile, e);
            std::process::exit(1);
        });

    // Use global local_embedding_model for local embedders, profile.embedding_model for others
    let embedding_model = config.resolve_embedding_model(profile);

    // Prepare shared services
    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let boot_core_instance = Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &embedding_model,
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(profile.default_collection_name.as_deref()),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        config.smart_routing_keys.clone(),
        config.ingestion.path_rules.clone(),
        config.ingestion.max_concurrent_requests,
        config.resolve_gpu_batch_size(profile, profile.default_collection_name.as_deref()),
        profile.num_ctx,
        file_detector.clone(),
        parser_factory.clone(),
    )
    .await
    .unwrap_or_else(|e| {
        eprintln!("Failed to initialize Core: {}", e);
        std::process::exit(1);
    });

    let boot_core = Arc::new(boot_core_instance);
    let boot_key = CoreKey::from_resolved(profile, &config);

    // Build the factory for lazy Core creation when a request arrives for a
    // collection that uses a different profile than the boot profile.
    let factory = CoreFactory {
        fastembed_cache_path: config.fastembed_cache_path.clone(),
        smart_routing_keys: config.smart_routing_keys.clone(),
        path_rules: config.ingestion.path_rules.clone(),
        max_concurrent_requests: config.ingestion.max_concurrent_requests,
        file_detector: file_detector.clone(),
        parser_factory: parser_factory.clone(),
    };

    let registry = Arc::new(CoreRegistry::new(
        boot_core,
        boot_key,
        target_profile.clone(),
        factory,
    ));
    let config = Arc::new(config);

    if args.stdio {
        run_stdio_server(registry, config, args.allow_local_fs, target_profile).await
    } else {
        vecdb_server::server::run_http_server(
            registry,
            config,
            args.allow_local_fs,
            target_profile,
            args.port,
        )
        .await
    }
}

async fn run_stdio_server(
    registry: Arc<vecdb_server::core_registry::CoreRegistry>,
    config: Arc<Config>,
    allow_local_fs: bool,
    target_profile: String,
) -> anyhow::Result<()> {
    if vecdb_common::OUTPUT.is_interactive {
        eprintln!("vecdb-mcp server running on stdio (Manual JSON-RPC)...");
        if allow_local_fs {
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
    let mut writer = stdout;

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        // Parse Request
        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                eprintln!("Invalid JSON-RPC request: {}", e);
                continue;
            }
        };

        // Handle Method
        let result = handle_request(&registry, &config, &req, allow_local_fs, &target_profile).await;

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
