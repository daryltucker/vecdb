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
            // ORT logging is silenced by default for two reasons:
            //   1. `ort::ep` shouts ERROR every time it probes a candidate path
            //      for `libonnxruntime_providers_shared.so` and one fails — even
            //      when it goes on to find the lib via RPATH and CUDA inits fine.
            //   2. `ort::logging` shouts ERROR on every aborted inference, e.g.
            //      "SkipLayerNormalization ... GetElementType is not implemented"
            //      when the user Ctrl+Cs an in-flight embed. The Future was just
            //      dropped — there's no real failure to report.
            // Genuine ORT failures already surface to the user via the Result<>
            // chain from embed()/embed_batch() (wrapped by `wrap_cuda_error` and
            // anyhow's `context`). LocalEmbedder also prints its own
            // "⚠️ [CUDA WARNING] GPU requested but ORT fell back to CPU"
            // diagnostic on init fallback. So silencing ORT's tracing layer does
            // NOT hide real issues; it just stops double-reporting them as noise.
            // Debug ORT directly with `RUST_LOG=ort=debug`.
            EnvFilter::new("error,vecdb=info,vecdb_core=info,vecdb_server=info,docsize=info,vecq=info,ort=off,onnxruntime=off,reqwest=error")
        })
    };

    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
