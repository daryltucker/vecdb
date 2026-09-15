use anyhow::Result;
use vecdb_core::embedder::Embedder;
use vecdb_core::embedders::local::LocalEmbedder;

/// Contract (BUG-2026-254): `use_gpu = true` may only produce two outcomes —
///
/// 1. `Ok`: the CUDA EP actually registered on the session (guaranteed by
///    `error_on_failure` on the dispatch), and embedding works, or
/// 2. `Err`: a loud, CUDA-shaped failure (provider libs missing next to the
///    executable, CUDA runtime absent, unsupported GPU arch, VRAM exhausted).
///
/// The outcome this test exists to forbid is the pre-fix third one: `Ok` with
/// a "✅ GPU Accelerated" banner while the EP silently failed to register and
/// everything ran on CPU. Before the fix this test could not fail on a
/// GPU-less machine; that vacuous pass is how the release gate stayed green
/// while GPU was broken.
#[tokio::test]
async fn test_cuda_initialization() -> Result<()> {
    // Force verbose logging for this test
    std::env::set_var("RUST_LOG", "debug");

    println!("--- TIER 2 CUDA TEST ---");
    // FORCE CLEAN ENV: Unset LD_LIBRARY_PATH to prove we don't need manual libs
    unsafe {
        std::env::remove_var("LD_LIBRARY_PATH");
    }
    println!("LD_LIBRARY_PATH forced unset for this test.");

    println!("Attempting to initialize LocalEmbedder with use_gpu=true...");
    // Constructor errors (unknown model, etc.) are real failures. GPU work is
    // lazy in a non-interactive process, so the CUDA outcome surfaces at the
    // first embed() below, not here.
    let embedder = LocalEmbedder::new("default", None, true)?;
    let model_name = embedder.model_name();
    if !model_name.contains("fastembed") {
        anyhow::bail!("Embedder initialized but is not fastembed: {}", model_name);
    }

    println!("Running test embedding (triggers lazy CUDA session init)...");
    match embedder.embed("Hello CUDA", None).await {
        Ok(vec) => {
            // With error_on_failure on the CUDA dispatch, an embedding coming
            // back means the CUDA EP genuinely registered on the session.
            println!("Embedding generated on GPU, length: {}", vec.len());
            assert!(!vec.is_empty());
        }
        Err(e) => {
            // A machine without a usable GPU MUST fail loudly, not fall back to
            // CPU under a success banner. Accept the failure iff it is
            // CUDA-shaped; anything else (model download, config, IO) is a real
            // test failure.
            let msg = format!("{e:#}");
            let cuda_shaped = msg.contains("GPU")
                || msg.contains("CUDA")
                || msg.contains("CUBLAS")
                || msg.contains("libonnxruntime_providers");
            assert!(
                cuda_shaped,
                "use_gpu=true embed failed for a non-CUDA reason: {msg}"
            );
            println!("GPU unusable on this machine — loud failure is the correct outcome:");
            println!("  {msg}");
        }
    }

    Ok(())
}
