//! Typed resource arbitration for embedders.
//!
//! Solves the structural problem behind the EXTERNAL_OLLAMA-blocks-local-GPU bug:
//! every embedder declares the resources it needs, and the `ResourceArbiter`
//! ensures concurrent embed calls only contend on resources they actually share.
//!
//! Two-tier scheme:
//!
//! * **In-process** — one `tokio::sync::Semaphore` per `Resource` value. Different
//!   resources have different semaphores, so they cannot block one another.
//!   Same-resource calls serialise according to permit count (`LocalGpu` = 1,
//!   `OllamaEndpoint` = 4 default, `LocalCpu` / `QdrantEndpoint` = unbounded).
//!
//! * **Cross-process** — for `LocalGpu` only, an exclusive advisory `flock` on
//!   `$XDG_RUNTIME_DIR/vecdb/locks/gpu.{device}.lock`. Two `vecdb` subprocesses
//!   on the same host cooperate instead of fighting each other through CUDA OOM.
//!   The lock is released automatically when the FD closes (process exit, panic,
//!   or end of permit scope).
//!
//! Acquisition is in stable order (sorted by resource discriminant + key) so
//! multi-resource permits cannot deadlock against each other.
//!
//! Background and design rationale: docs/planning/BUG_IDLE_VRAM_AND_RESOURCE_ISOLATION.md

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use fs2::FileExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A named resource that an embedder may need to do work.
///
/// Variants correspond to physical or logical bottlenecks. Two `Resource`
/// values that compare equal share the same semaphore (and, for `LocalGpu`,
/// the same flock file). Variants are designed so that `EXTERNAL_OLLAMA` and
/// local-GPU work cannot share a semaphore — that is the structural fix for
/// the prior cross-profile blocking bug.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Resource {
    /// A specific local GPU device. Single-tenant within a process; cooperates
    /// across processes via flock.
    LocalGpu { device: u32 },
    /// Local CPU. Unbounded — the OS scheduler is the arbiter.
    LocalCpu,
    /// A remote Ollama endpoint. Bounded per-URL by `default_permits()`; allows
    /// multiple concurrent in-flight requests up to that limit.
    OllamaEndpoint { url: String },
    /// A remote Qdrant endpoint. Unbounded — Qdrant handles its own concurrency.
    QdrantEndpoint { url: String },
}

impl Resource {
    /// Permit budget for this resource if not overridden by the arbiter caller.
    fn default_permits(&self) -> usize {
        match self {
            Resource::LocalGpu { .. } => 1,
            Resource::LocalCpu => Semaphore::MAX_PERMITS,
            Resource::OllamaEndpoint { .. } => 4,
            Resource::QdrantEndpoint { .. } => Semaphore::MAX_PERMITS,
        }
    }

    /// True if this resource needs cross-process arbitration (a file lock)
    /// in addition to the in-process semaphore.
    fn needs_file_lock(&self) -> bool {
        matches!(self, Resource::LocalGpu { .. })
    }

    /// File path used for the cross-process advisory lock. Only meaningful when
    /// `needs_file_lock()` is true. Located under `$XDG_RUNTIME_DIR/vecdb/locks/`
    /// (tmpfs, auto-cleared on logout) with a fallback to `~/.cache/vecdb/locks/`.
    fn lock_path(&self) -> Option<PathBuf> {
        let Resource::LocalGpu { device } = self else {
            return None;
        };
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .or_else(dirs::cache_dir)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Some(
            base.join("vecdb")
                .join("locks")
                .join(format!("gpu.{device}.lock")),
        )
    }

    /// Stable ordering key used to acquire multi-resource permit sets in a
    /// fixed order, eliminating lock-order deadlocks.
    fn ord_key(&self) -> (u8, String) {
        match self {
            Resource::LocalGpu { device } => (0, format!("{device}")),
            Resource::OllamaEndpoint { url } => (1, url.clone()),
            Resource::QdrantEndpoint { url } => (2, url.clone()),
            Resource::LocalCpu => (3, String::new()),
        }
    }
}

/// RAII guard for a held resource permit. Releases all underlying permits and
/// file locks on drop.
///
/// `embed()` flow:
/// ```text
/// let _g = arbiter.acquire(&embedder.required_resources()).await?;
/// model.embed(...)?;
/// // _g drops here → semaphore permit released, flock released
/// ```
pub struct ResourcePermit {
    _semaphore_permits: Vec<OwnedSemaphorePermit>,
    _file_locks: Vec<FileLockGuard>,
}

/// Owns an open `File` whose advisory lock is released on drop.
struct FileLockGuard {
    file: File,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        // Best-effort. flock release on FD close is the real safety net.
        let _ = FileExt::unlock(&self.file);
    }
}

/// Per-process arbiter. Lazily creates a semaphore per distinct resource on first use.
pub struct ResourceArbiter {
    semaphores: tokio::sync::Mutex<HashMap<Resource, Arc<Semaphore>>>,
    /// Permit overrides for specific resources. Most callers leave this empty
    /// and rely on `Resource::default_permits()`.
    overrides: HashMap<Resource, usize>,
}

impl Default for ResourceArbiter {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceArbiter {
    pub fn new() -> Self {
        Self {
            semaphores: tokio::sync::Mutex::new(HashMap::new()),
            overrides: HashMap::new(),
        }
    }

    /// Override the permit count for a specific resource. Useful for tests
    /// and for deployments where Ollama's `OLLAMA_NUM_PARALLEL` is tuned high.
    pub fn with_override(mut self, resource: Resource, permits: usize) -> Self {
        self.overrides.insert(resource, permits);
        self
    }

    /// Acquire permits for every resource in `resources`, returning a single
    /// guard whose Drop releases all of them.
    ///
    /// Resources are acquired in stable order to prevent deadlocks. Duplicates
    /// in the input are collapsed — asking for `[LocalGpu]` twice still only
    /// holds one permit.
    ///
    /// For each `LocalGpu` resource, an exclusive `flock` is also acquired
    /// before the semaphore permit, so two processes contending the same GPU
    /// queue cleanly instead of crashing in CUDA.
    pub async fn acquire(&self, resources: &[Resource]) -> Result<ResourcePermit> {
        // Dedupe + sort by stable key so lock acquire order is deterministic
        // regardless of the order callers pass resources.
        let mut sorted: Vec<Resource> = resources.to_vec();
        sorted.sort_by_key(|r| r.ord_key());
        sorted.dedup();

        let mut sem_permits = Vec::with_capacity(sorted.len());
        let mut file_locks = Vec::new();

        for resource in &sorted {
            // 1. Cross-process lock first (where applicable). This blocks the
            //    *thread* — but we run it via spawn_blocking so the runtime stays
            //    responsive. Order matters: take the file lock before the
            //    in-process semaphore so a single-process workload doesn't
            //    pointlessly serialize on flock.
            //    Actually the opposite: take the in-process semaphore first
            //    (cheap, async), then escalate to flock. This avoids holding a
            //    cross-process lock while waiting on in-process queue.
            //    Decision: in-process first.

            let sem = self.semaphore_for(resource).await;
            let permit = sem
                .acquire_owned()
                .await
                .context("Semaphore closed unexpectedly")?;
            sem_permits.push(permit);

            if resource.needs_file_lock() {
                let lock = acquire_file_lock(resource).await?;
                file_locks.push(lock);
            }
        }

        Ok(ResourcePermit {
            _semaphore_permits: sem_permits,
            _file_locks: file_locks,
        })
    }

    async fn semaphore_for(&self, resource: &Resource) -> Arc<Semaphore> {
        let mut map = self.semaphores.lock().await;
        if let Some(sem) = map.get(resource) {
            return sem.clone();
        }
        let permits = self
            .overrides
            .get(resource)
            .copied()
            .unwrap_or_else(|| resource.default_permits());
        let sem = Arc::new(Semaphore::new(permits));
        map.insert(resource.clone(), sem.clone());
        sem
    }
}

/// Open and exclusively lock the file for `resource`. Runs the blocking flock
/// call on a dedicated thread so the Tokio runtime stays responsive even when
/// another process holds the lock for a long time.
async fn acquire_file_lock(resource: &Resource) -> Result<FileLockGuard> {
    let path = resource
        .lock_path()
        .ok_or_else(|| anyhow::anyhow!("Resource {:?} does not support file locking", resource))?;

    tokio::task::spawn_blocking(move || {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create lock dir {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open lock file {}", path.display()))?;

        // Exclusive blocking lock. Released on FD drop.
        FileExt::lock_exclusive(&file).with_context(|| format!("flock {}", path.display()))?;

        Ok(FileLockGuard { file })
    })
    .await
    .context("Lock task panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn test_different_resources_do_not_block() {
        let arb = Arc::new(ResourceArbiter::new());

        // Hold one resource for a clearly observable interval...
        let arb_a = arb.clone();
        let a = tokio::spawn(async move {
            let _g = arb_a
                .acquire(&[Resource::OllamaEndpoint {
                    url: "remote".into(),
                }])
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
        });

        // ...and confirm an unrelated resource's acquire does not wait for it.
        //
        // The property is "did not block", which is observable directly: if the
        // acquire returned without waiting, the 150ms holder is necessarily
        // still running. Asserting that instead of an absolute duration is what
        // makes this deterministic.
        //
        // It used to assert `elapsed < 200ms`. That is a wall-clock measurement
        // taken inside a gate that runs test binaries CONCURRENTLY, so it
        // reported machine load as much as arbiter behaviour — the same reason
        // `make test-perf` exists as a separate, serial target. Observed
        // failing at 308ms under full-suite load on day 238 while passing 5/5
        // in isolation on the same commit.
        let start = Instant::now();
        let _g = arb
            .acquire(&[Resource::LocalGpu { device: 99 }])
            .await
            .unwrap();
        let elapsed = start.elapsed();

        assert!(
            !a.is_finished(),
            "acquiring an unrelated resource waited for the held one to be \
             released (took {elapsed:?}) — the arbiter is serialising resources \
             that share nothing"
        );

        a.await.unwrap();

        // The absolute bound is still worth having, but only where a clock is
        // meaningful: serially, under `make test-perf`, like every other timing
        // assertion in this repo.
        if std::env::var("VECDB_PERF_ASSERT").is_ok() {
            assert!(
                elapsed < Duration::from_millis(200),
                "unrelated resources should not block; took {elapsed:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_same_resource_serialises() {
        let arb = Arc::new(ResourceArbiter::new());
        let inflight = Arc::new(AtomicUsize::new(0));
        let max_inflight = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];
        for _ in 0..5 {
            let arb = arb.clone();
            let inflight = inflight.clone();
            let max_inflight = max_inflight.clone();
            handles.push(tokio::spawn(async move {
                let _g = arb
                    .acquire(&[Resource::LocalGpu { device: 0 }])
                    .await
                    .unwrap();
                let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max_inflight.fetch_max(cur, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                inflight.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(
            max_inflight.load(Ordering::SeqCst),
            1,
            "LocalGpu has 1 permit; never more than one task in critical section"
        );
    }

    #[tokio::test]
    async fn test_local_cpu_is_unbounded() {
        let arb = Arc::new(ResourceArbiter::new());
        let inflight = Arc::new(AtomicUsize::new(0));
        let max_inflight = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];
        for _ in 0..16 {
            let arb = arb.clone();
            let inflight = inflight.clone();
            let max_inflight = max_inflight.clone();
            handles.push(tokio::spawn(async move {
                let _g = arb.acquire(&[Resource::LocalCpu]).await.unwrap();
                let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max_inflight.fetch_max(cur, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                inflight.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert!(
            max_inflight.load(Ordering::SeqCst) >= 8,
            "LocalCpu should let many tasks through concurrently"
        );
    }

    #[tokio::test]
    async fn test_ollama_endpoint_default_permits() {
        let arb = Arc::new(ResourceArbiter::new());
        let inflight = Arc::new(AtomicUsize::new(0));
        let max_inflight = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];
        for _ in 0..10 {
            let arb = arb.clone();
            let inflight = inflight.clone();
            let max_inflight = max_inflight.clone();
            handles.push(tokio::spawn(async move {
                let _g = arb
                    .acquire(&[Resource::OllamaEndpoint { url: "x".into() }])
                    .await
                    .unwrap();
                let cur = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                max_inflight.fetch_max(cur, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(30)).await;
                inflight.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(
            max_inflight.load(Ordering::SeqCst),
            4,
            "OllamaEndpoint default is 4 permits — exactly that many in flight"
        );
    }

    #[tokio::test]
    async fn test_multi_resource_dedup_and_order() {
        // Asking for the same resource twice must still only hold one permit
        // and must not deadlock against itself.
        let arb = Arc::new(ResourceArbiter::new());
        let r = Resource::LocalGpu { device: 0 };
        let _g = tokio::time::timeout(Duration::from_secs(1), arb.acquire(&[r.clone(), r.clone()]))
            .await
            .expect("must not deadlock on duplicates")
            .expect("must succeed");
    }

    /// Regression test for the user's reported bug.
    ///
    /// EXTERNAL_OLLAMA ingest (network-only) running concurrently with a
    /// local-GPU ingest must NOT block each other — they touch different
    /// resources and so must run in parallel.
    #[tokio::test]
    async fn test_external_ollama_does_not_block_local_gpu() {
        let arb = Arc::new(ResourceArbiter::new());
        // Barrier proves both tasks hold their locks simultaneously.
        // If either resource serialised the other, task B would wait for task A
        // to release before acquiring — and could never reach the barrier while
        // task A is still inside its critical section.
        let barrier = Arc::new(tokio::sync::Barrier::new(2));

        let arb_a = arb.clone();
        let bar_a = barrier.clone();
        let task_a = tokio::spawn(async move {
            let _g = arb_a
                .acquire(&[Resource::OllamaEndpoint {
                    url: "http://external:11434".into(),
                }])
                .await
                .unwrap();
            bar_a.wait().await; // rendezvous while holding the lock
        });

        let arb_b = arb.clone();
        let bar_b = barrier.clone();
        let task_b = tokio::spawn(async move {
            let _g = arb_b
                .acquire(&[Resource::LocalGpu { device: 0 }])
                .await
                .unwrap();
            bar_b.wait().await; // rendezvous while holding the lock
        });

        // If resources contend, one task blocks on acquire and never reaches the
        // barrier — tokio::join! deadlocks. Add a timeout so the test fails fast
        // with a clear message instead of hanging the suite.
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(task_a, task_b)
        })
        .await;

        assert!(
            result.is_ok(),
            "EXTERNAL_OLLAMA must not block local GPU — tasks deadlocked, suggesting serialisation"
        );
    }
}
