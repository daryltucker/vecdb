use tracing_subscriber::{fmt, EnvFilter};

/// Initializes logging for vecdb binaries.
///
/// Default behavior:
/// - If VECDB_DEBUG is set, sets level to `debug` for all components.
/// - Otherwise, sets `warn` default, with `info` for vecdb crates.
/// - Respects RUST_LOG if present.
/// - Always outputs to stderr to avoid polluting stdout (critical for MCP/Pipe modes).
pub fn init_logging() {
    let filter = if std::env::var("VECDB_DEBUG").is_ok() {
        EnvFilter::new("debug")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            // Default: GLOBAL=error, ours=info
            // Explicitly silence noisy external crates even at info
            EnvFilter::new("error,vecdb=info,vecdb_core=info,vecdb_server=info,docsize=info,vecq=info,ort=error,reqwest=error,onnxruntime=error")
        })
    };

    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
