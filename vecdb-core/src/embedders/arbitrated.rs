//! `ArbitratedEmbedder` — a decorator that wraps any `Embedder` and acquires
//! resource permits from a `ResourceArbiter` before delegating embed calls.
//!
//! Wiring: `Core::new` constructs the inner embedder, then wraps it before
//! storing. All consumer code (ingestion pipeline, history, search) calls
//! through this wrapper transparently.
//!
//! Decorator over inheritance: keeps each concrete embedder focused on the
//! engine it talks to. The arbitration concern lives in exactly one place.

use crate::embedder::Embedder;
use crate::resource::{Resource, ResourceArbiter};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

pub struct ArbitratedEmbedder {
    inner: Arc<dyn Embedder + Send + Sync>,
    arbiter: Arc<ResourceArbiter>,
}

impl ArbitratedEmbedder {
    pub fn new(
        inner: Arc<dyn Embedder + Send + Sync>,
        arbiter: Arc<ResourceArbiter>,
    ) -> Self {
        Self { inner, arbiter }
    }

    /// Borrow the underlying embedder. Used in tests and for diagnostics.
    pub fn inner(&self) -> &Arc<dyn Embedder + Send + Sync> {
        &self.inner
    }
}

#[async_trait]
impl Embedder for ArbitratedEmbedder {
    async fn embed(&self, text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        let resources = self.inner.required_resources();
        // No resources declared → fast path, skip arbitration entirely.
        if resources.is_empty() {
            return self.inner.embed(text, target_dim).await;
        }
        let _permit = self.arbiter.acquire(&resources).await?;
        self.inner.embed(text, target_dim).await
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let resources = self.inner.required_resources();
        if resources.is_empty() {
            return self.inner.embed_batch(texts, target_dim).await;
        }
        let _permit = self.arbiter.acquire(&resources).await?;
        self.inner.embed_batch(texts, target_dim).await
    }

    async fn dimension(&self) -> Result<usize> {
        // dimension() may be called eagerly at startup ("probe") and is cheap
        // for most embedders. We acquire too — a probe and an embed should
        // serialise on the same resource so the probe doesn't OOM-fight a
        // running embed.
        let resources = self.inner.required_resources();
        if resources.is_empty() {
            return self.inner.dimension().await;
        }
        let _permit = self.arbiter.acquire(&resources).await?;
        self.inner.dimension().await
    }

    fn model_name(&self) -> String {
        self.inner.model_name()
    }

    fn release(&self) {
        self.inner.release();
    }

    fn required_resources(&self) -> Vec<Resource> {
        self.inner.required_resources()
    }
}
