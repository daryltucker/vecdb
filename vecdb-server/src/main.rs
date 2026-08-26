/*
 * PURPOSE:
 *   Main initialization for the MCP Server.
 *   Hosts the Model Context Protocol interface via manual JSON-RPC.
 *   (Replaces SDK approach for reliability and speed)
 */

use clap::Parser;

use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecdb_core::Core;
use vecdb_server::core_registry::{
    start_watchdog, CoreFactory, CoreKey, CoreRegistry, EvictionMode,
};
use vecdb_server::rpc::{
    handle_request,
    types::{JsonRpcError, JsonRpcRequest, JsonRpcResponse},
};
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
        let version = env!("CARGO_PKG_VERSION");
        // Stamped at build time — see vecdb-common/build.rs.
        let git_hash = vecdb_common::revision();

        println!("vecdb-server {} (git:{})", version, git_hash);
        // Also show key config paths
        if let Ok(config_path) = Config::get_path() {
            eprintln!("Config: {}", config_path.display());
        }
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

    let resolution = config
        .resolve(Some(&target_profile), None)
        .unwrap_or_else(|e| {
            eprintln!("Error resolving profile '{}': {}", target_profile, e);
            std::process::exit(1);
        });

    // Prepare shared services
    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    // Don't eagerly load GPU at boot. The server creates a boot Core for the default
    // profile (which may use local GPU embedding). If this server only serves requests
    // for remote-Ollama collections, that GPU memory sits unused and blocks other
    // processes from using the GPU.
    // VECDB_SKIP_PROBE defers embedder initialization to the first actual embed() call,
    // so GPU is only loaded when a request needs local embedding.
    // If no local-embed requests ever arrive, GPU stays free for other processes.
    std::env::set_var("VECDB_SKIP_PROBE", "1");

    // One set of services, shared by the boot Core and every Core the factory
    // builds later — so they cannot drift apart.
    let services = vecdb_core::CoreServices::from_config(
        &config,
        file_detector.clone(),
        parser_factory.clone(),
    );

    let boot_core_instance = Core::new(&resolution, services.clone())
        .await
        .unwrap_or_else(|e| {
            eprintln!("Failed to initialize Core: {}", e);
            std::process::exit(1);
        });

    let boot_core = Arc::new(boot_core_instance);
    let boot_key = CoreKey::from_resolution(&resolution);

    // Build the factory for lazy Core creation when a request arrives for a
    // collection that uses a different profile than the boot profile.
    let factory = CoreFactory { services };

    let registry = Arc::new(CoreRegistry::new(
        boot_core,
        boot_key,
        target_profile.clone(),
        factory,
        config.clone(),
    ));

    // Idle-eviction watchdog. Stdio subprocesses exit on deep-idle so the OS reclaims
    // the residual CUDA context (~80 MiB) — the MCP client respawns them on next use.
    // HTTP daemons stay up; deep-idle just drops the cache entry.
    let eviction_mode = if args.stdio {
        EvictionMode::ExitOnDeepIdle
    } else {
        EvictionMode::CacheOnly
    };
    let shutdown_rx = start_watchdog(registry.clone(), config.server.clone(), eviction_mode);

    let config = Arc::new(config);

    if args.stdio {
        run_stdio_server(
            registry,
            config,
            args.allow_local_fs,
            target_profile,
            shutdown_rx,
        )
        .await
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
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
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

    loop {
        // Race the next stdin line against a shutdown signal from the watchdog.
        // On deep-idle, the watchdog flips the channel and we exit cleanly so the
        // OS reclaims the process-global CUDA context. The MCP client respawns
        // this subprocess on its next request.
        let line = tokio::select! {
            biased; // prefer shutdown over new work if both are ready
            res = shutdown_rx.changed() => {
                if res.is_err() || *shutdown_rx.borrow() {
                    if vecdb_common::OUTPUT.is_interactive {
                        eprintln!("vecdb-mcp: deep-idle reached, exiting cleanly");
                    }
                    break;
                }
                continue;
            }
            line = reader.next_line() => match line? {
                Some(l) => l,
                None => break, // EOF — client disconnected
            },
        };

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

        // Handle Method (inside a spawned task so panics don't kill the server).
        // tokio::task::spawn catches panics and surfaces them as JoinError.
        // This is our panic boundary — if ANY tool handler panics (e.g. bug in a
        // dependency, weird file path, OOM), the server logs the panic and returns
        // a JSON-RPC error instead of crashing the process.
        let registry_t = registry.clone();
        let config_t = config.clone();
        let tp_t = target_profile.clone();
        let method_t = req.method.clone();
        let params_t = req.params.clone();
        let id_t = req.id.clone();

        let handle = tokio::task::spawn(async move {
            let req = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: method_t,
                params: params_t,
                id: id_t,
            };
            handle_request(&registry_t, &config_t, &req, allow_local_fs, &tp_t).await
        });

        let result = match handle.await {
            Ok(result) => result,
            Err(e) if e.is_panic() => {
                let msg = e
                    .into_panic()
                    .downcast::<String>()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| "unknown panic".to_string());
                eprintln!("[vecdb-mcp] PANIC in tool handler: {msg}");
                Err(JsonRpcError {
                    code: -32000,
                    message: format!("Internal error (panic): {msg}"),
                    data: None,
                })
            }
            Err(_) => {
                // Task cancelled — server is shutting down.
                continue;
            }
        };

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
