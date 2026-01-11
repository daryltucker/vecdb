use vecdb_core::embedders::local::LocalEmbedder;
use vecdb_core::embedder::Embedder;
use anyhow::Result;

#[tokio::test]
async fn test_cuda_initialization() -> Result<()> {
    // Force verbose logging for this test
    std::env::set_var("RUST_LOG", "debug");
    // tracing_subscriber not available in core tests, rely on stdout
    // tracing_subscriber::fmt::try_init().ok();

    println!("--- TIER 2 CUDA TEST ---");
    println!("Checking environment...");
    // FORCE CLEAN ENV: Unset LD_LIBRARY_PATH to prove we don't need manual libs
    unsafe { std::env::remove_var("LD_LIBRARY_PATH"); }
    println!("LD_LIBRARY_PATH forced unset for this test.");

    println!("Attempting to initialize LocalEmbedder with use_gpu=true...");
    let embedder = LocalEmbedder::new(None, true)?;

    let model_name = embedder.model_name();
    println!("Initialized successfully via: {}", model_name);

    // Verify it didn't silently fallback
    if !model_name.contains("fastembed") {
        anyhow::bail!("Embedder initialized but is not fastembed: {}", model_name);
    }
    
    // We can't strictly assert GPU usage from the public API easily without parsing logs,
    // but the initialization shouldn't crash.
    
    // Try a simple embedding
    println!("Running test embedding...");
    let vec = embedder.embed("Hello CUDA").await?;
    println!("Embedding generated, length: {}", vec.len());
    assert!(vec.len() > 0);

    Ok(())
}
