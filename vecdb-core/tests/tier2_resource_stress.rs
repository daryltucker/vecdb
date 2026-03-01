use std::sync::Arc;
use tokio::time::{timeout, Duration};
use vecdb_core::embedder::Embedder;
use vecdb_core::embedders::LocalEmbedder;

// ═══════════════════════════════════════════════════════════════
// TIER 2: INTEGRATION — Embedding Stress Test
// ═══════════════════════════════════════════════════════════════
//
// PROGRESSIVE TRUST:
//   This test proves that heavy embedding batches don't hang, OOM,
//   or deadlock. Without this proof, Tier 3+ tests that embed real
//   data (100+ files) cannot be trusted — a hang here would cause
//   a 60-minute timeout there.
//
// PREREQUISITE: T1 unit tests pass (proven: embedder initializes)
// OPENS GATE FOR: T3/T4 tests that embed real content
//
// DATA: 50 chunks × 4KB = ~200KB synthetic batch
// TIME BUDGET: < 60s (generous; should finish in < 10s)
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_heavy_batch_embedding_stress() {
    // SCENARIO: Simulating a heavy flush_chunks payload
    // 20 chunks * 6000 chars = 120KB batch.
    // This previously caused 100% CPU lockups if ONNX threads weren't capped.

    // 1. Setup
    // Initialize real LocalEmbedder (will download model if needed, but usually cached)
    let embedder = LocalEmbedder::new("default", None, false).expect("Failed to init embedder");
    let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(embedder);

    // 2. Generate Heavy Load
    // 50 chunks of 4KB random-ish text
    let mut texts = Vec::new();
    let base_text = "Standard Lorem Ipsum ".repeat(200); // ~4KB
    for i in 0..50 {
        texts.push(format!("Job {} {}", i, base_text));
    }

    println!(
        "Starting heavy batch embedding of {} items (approx {} KB total)...",
        texts.len(),
        (texts.len() * base_text.len()) / 1024
    );

    // 3. Execution with Timeout
    // If resource starvation occurs, this might hang or take forever.
    // We enforce a generous timeout (60s) just to be safe, but it should finish much faster.
    let task = async {
        let start = std::time::Instant::now();
        let _ = embedder.embed_batch(&texts, None).await.expect("Batch failed");
        start.elapsed()
    };

    let duration = timeout(Duration::from_secs(60), task).await;

    match duration {
        Ok(d) => println!("Success! Heavy batch finished in {:.2?}", d),
        Err(_) => panic!("Heavy batch timed out! Potential resource starvation or deadlock."),
    }
}
