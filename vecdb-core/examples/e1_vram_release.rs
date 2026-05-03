// E1: Does LocalEmbedder::release_gpu() actually return VRAM mid-process?
//
// Run with:  cargo run --example e1_vram_release --features cuda --release
//
// Reports VRAM held by *this* PID at each phase. If "after release" drops to 0
// (or near-0) while the process is still alive, idle eviction can be in-process.
// If it stays pinned, idle eviction must escalate to subprocess exit.

use std::process::Command;
use std::time::Duration;
use vecdb_core::embedders::local::LocalEmbedder;
use vecdb_core::embedder::Embedder;

fn vram_for_pid(pid: u32) -> String {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ])
        .output();
    match out {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            for line in s.lines() {
                let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
                if parts.len() == 2 && parts[0] == pid.to_string() {
                    return format!("{} MiB", parts[1]);
                }
            }
            "0 MiB (not listed)".to_string()
        }
        Err(e) => format!("nvidia-smi error: {e}"),
    }
}

fn phase(label: &str, pid: u32) {
    // Brief settle so the driver bookkeeping catches up.
    std::thread::sleep(Duration::from_millis(500));
    println!("[E1] {:<28} VRAM(pid {}): {}", label, pid, vram_for_pid(pid));
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pid = std::process::id();
    println!("[E1] PID = {pid}");
    println!("[E1] Model: all-minilm-l6-v2 (smallest, 22M params)");
    println!();

    phase("baseline (pre-construct)", pid);

    // Construct (lazy — should not allocate VRAM yet).
    let embedder = LocalEmbedder::new("all-minilm-l6-v2", None, true)?;
    phase("after construct (lazy)", pid);

    // Trigger init via a real embed call.
    let _ = embedder.embed("hello world", None).await?;
    phase("after first embed()", pid);

    // Embed a few more to make sure the session is warm and any deferred
    // allocations have happened.
    for i in 0..3 {
        let _ = embedder.embed(&format!("warmup {i}"), None).await?;
    }
    phase("after warmup embeds", pid);

    // The moment of truth.
    embedder.release_gpu();
    phase("immediately after release", pid);

    // Give the driver a chance to actually reclaim.
    std::thread::sleep(Duration::from_secs(2));
    phase("2s after release", pid);

    std::thread::sleep(Duration::from_secs(3));
    phase("5s after release", pid);

    // Reload: embed again. If release truly worked, this re-allocates.
    let _ = embedder.embed("after release reload", None).await?;
    phase("after reload embed()", pid);

    println!();
    println!("[E1] Interpretation:");
    println!("  PASS  if 'after release' drops near 0 and 'after reload embed()' rises again.");
    println!("  FAIL  if VRAM stays pinned through the release phases.");
    Ok(())
}
