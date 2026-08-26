/*
 * PURPOSE:
 *   Concrete implementation of Embedder trait using Ollama API.
 *   Provides easy local embedding generation.
 *
 * REQUIREMENTS:
 *   - Connect to local/remote Ollama instance
 *   - Support configured model (default: nomic-embed-text)
 *   - Handle API errors gracefully
 *   - Use /api/embed endpoint (not deprecated /api/embeddings)
 *   - Refuse, rather than silently truncate, inputs exceeding model context
 *     (opt in with `with_truncation(true)` / `allow_embed_truncation`)
 */

use crate::embedder::Embedder;
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct OllamaEmbedder {
    client: Client,
    base_url: String,
    model: String,
    num_ctx: Option<usize>,
    /// Whether Ollama may silently cut inputs that exceed the model context.
    ///
    /// Off by default. Truncation at embed time is the last place content can
    /// disappear, and it is the least visible: the vector comes back the right
    /// shape, the upsert succeeds, and the missing tail is only discoverable by
    /// noticing a search that should have hit and didn't. Re-ingesting is the
    /// only repair, and nothing tells you that you need to.
    ///
    /// Refusing instead surfaces an oversized chunk as a chunking-configuration
    /// problem, which is what it actually is.
    truncate: bool,
}

#[derive(Serialize)]
struct OllamaOptions {
    num_ctx: usize,
}

/// Single-text embedding request using /api/embed
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
    /// Whether Ollama may cut input to fit the model's context window.
    /// When false (the default), inputs exceeding the model's context length
    /// (e.g. 512 tokens for nomic-embed-text-v2-moe) are a hard error — which
    /// is the point: an oversized chunk is a chunking-config bug, and losing its
    /// tail silently is strictly worse than being told about it.
    truncate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

/// Batch embedding request using /api/embed
#[derive(Serialize)]
struct EmbedBatchRequest<'a> {
    model: &'a str,
    input: &'a [String],
    truncate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

/// Response from /api/embed — always returns a list of embeddings
#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

/// Error response from Ollama
#[derive(Deserialize)]
struct OllamaError {
    error: String,
}

impl OllamaEmbedder {
    pub fn new(
        base_url: String,
        model: String,
        accept_invalid_certs: bool,
        api_key: Option<String>,
        num_ctx: Option<usize>,
    ) -> Self {
        let mut builder = reqwest::ClientBuilder::new()
            .danger_accept_invalid_certs(accept_invalid_certs)
            .timeout(std::time::Duration::from_secs(120)); // Strict 120s timeout prevents silent hangs

        if let Some(key) = api_key {
            // Create default headers with Authorization
            let mut headers = reqwest::header::HeaderMap::new();
            let mut auth_value = reqwest::header::HeaderValue::from_str(&format!("Bearer {}", key))
                .expect("Invalid API Key format");
            auth_value.set_sensitive(true);
            headers.insert(reqwest::header::AUTHORIZATION, auth_value);
            builder = builder.default_headers(headers);
        }

        let client = builder.build().expect("Failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            num_ctx,
            truncate: false,
        }
    }

    /// Permit Ollama to silently cut inputs exceeding the model context window.
    ///
    /// Opt-in. See the `truncate` field for why this is not the default.
    pub fn with_truncation(mut self, truncate: bool) -> Self {
        self.truncate = truncate;
        self
    }

    /// Parse an Ollama error response into a human-readable message.
    fn format_ollama_error(error_text: &str, context: &str) -> anyhow::Error {
        // Try to parse as JSON error
        if let Ok(err) = serde_json::from_str::<OllamaError>(error_text) {
            if err.error.contains("input length exceeds") || err.error.contains("context length") {
                return anyhow::anyhow!(
                    "Ollama embedding failed: input exceeds model context window.\n\
                     \n\
                     Model '{}' has a limited context window.\n\
                     To fix:\n\
                       1. Reduce target_chunk_size in config.toml (e.g., target_chunk_size = 256)\n\
                       2. Or use a model with longer context (e.g., nomic-embed-text-v1.5 has 8192 tok)\n\
                     \n\
                     Context: {}\n\
                     Raw error: {}",
                    context, context, err.error
                );
            }
            anyhow::anyhow!("Ollama API error: {}", err.error)
        } else {
            anyhow::anyhow!("Ollama API error: {}", error_text)
        }
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, text: &str, target_dim: Option<usize>) -> Result<Vec<f32>> {
        let url = format!("{}/api/embed", self.base_url);

        let request = EmbedRequest {
            model: &self.model,
            input: text,
            truncate: self.truncate,
            options: self.num_ctx.map(|ctx| OllamaOptions { num_ctx: ctx }),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Ollama network error or timeout. Ensure the server is reachable and processing requests.")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Self::format_ollama_error(&error_text, &self.model));
        }

        let embed_response: EmbedResponse = response
            .json()
            .await
            .context("Failed to parse Ollama /api/embed response")?;

        let mut vec = embed_response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Ollama returned no embeddings"))?;

        if let Some(dim) = target_dim {
            if dim < vec.len() {
                vec.truncate(dim);
                crate::embedder::l2_normalize(&mut vec);
            }
        }

        Ok(vec)
    }

    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.base_url);

        // Send the exact batch array dictated by pipeline.rs / gpu_concurrency config
        let request = EmbedBatchRequest {
            model: &self.model,
            input: texts,
            truncate: self.truncate,
            options: self.num_ctx.map(|ctx| OllamaOptions { num_ctx: ctx }),
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Ollama network error or timeout. Ensure the server is reachable and processing requests.")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Self::format_ollama_error(&error_text, &self.model));
        }

        let embed_response: EmbedResponse = response
            .json()
            .await
            .context("Failed to parse Ollama /api/embed batch response")?;

        let mut results = embed_response.embeddings;

        if let Some(dim) = target_dim {
            for vec in results.iter_mut() {
                if dim < vec.len() {
                    vec.truncate(dim);
                    crate::embedder::l2_normalize(vec);
                }
            }
        }

        Ok(results)
    }

    async fn dimension(&self) -> Result<usize> {
        // Probe via a small embedding to get dimension
        let dummy = self.embed("probe", None).await?;
        Ok(dummy.len())
    }

    fn model_name(&self) -> String {
        format!("ollama:{}", self.model)
    }

    /// Build a full `ModelIdentity` from `/api/show` plus `/api/tags`.
    ///
    /// Two calls because they carry different things: `/api/show` has the
    /// architecture, parameter size, quantization level and the real context
    /// and embedding lengths; only `/api/tags` carries the digest, which is the
    /// one field that is actually identity. A tag string is not — on blade,
    /// `qwen3-embedding:4b` and `qwen3-embedding:4b-q4_K_M` are the same blob
    /// while `4b-q8_0` is different weights.
    ///
    /// A failure here is not fatal: it degrades to whatever was collected. The
    /// compatibility guard refuses on insufficient identity, so a partial
    /// answer produces a refusal to write, never a silent bad write.
    async fn identity(&self) -> Result<crate::types::ModelIdentity> {
        let mut identity = crate::types::ModelIdentity::unknown(self.model.clone());

        if let Ok(resp) = self
            .client
            .post(format!("{}/api/show", self.base_url))
            .json(&serde_json::json!({ "model": self.model }))
            .send()
            .await
        {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let details = &body["details"];
                identity.family = details["family"].as_str().map(str::to_string);
                identity.parameter_size = details["parameter_size"].as_str().map(str::to_string);
                identity.quantization_level =
                    details["quantization_level"].as_str().map(str::to_string);

                let info = &body["model_info"];
                identity.architecture = info["general.architecture"].as_str().map(str::to_string);

                // The embedding_length / context_length keys are namespaced by
                // architecture (e.g. "qwen3.embedding_length"), so the arch has
                // to be known before they can be addressed.
                if let Some(arch) = &identity.architecture {
                    identity.embedding_length = info[format!("{arch}.embedding_length")].as_u64();
                    identity.context_length = info[format!("{arch}.context_length")].as_u64();
                }
            }
        }

        if let Ok(resp) = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(models) = body["models"].as_array() {
                    // Ollama reports bare tags as "name:latest"; match either form
                    // so a configured "qwen3-embedding:0.6b" finds its entry.
                    identity.digest = models
                        .iter()
                        .find(|m| {
                            m["name"].as_str().is_some_and(|n| {
                                n == self.model || n.trim_end_matches(":latest") == self.model
                            })
                        })
                        .and_then(|m| m["digest"].as_str())
                        .map(str::to_string);
                }
            }
        }

        Ok(identity)
    }

    fn required_resources(&self) -> Vec<crate::resource::Resource> {
        // Keyed on the base URL — two profiles pointing at the same endpoint
        // share the per-endpoint permit budget; profiles pointing at different
        // endpoints don't contend at all.
        vec![crate::resource::Resource::OllamaEndpoint {
            url: self.base_url.clone(),
        }]
    }
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    fn embedder() -> OllamaEmbedder {
        OllamaEmbedder::new(
            "http://localhost:11434".to_string(),
            "nomic-embed-text".to_string(),
            false,
            None,
            None,
        )
    }

    /// Regression: `truncate` was hardcoded `true` in both request builders.
    ///
    /// That made embed-time truncation the last and quietest place ingested
    /// content could vanish — the vector comes back the right shape, the upsert
    /// succeeds, and the missing tail is only discoverable by noticing a search
    /// that should have hit and didn't. Re-ingesting is the only repair and
    /// nothing tells you that you need to.
    #[test]
    fn truncation_is_off_unless_asked_for() {
        assert!(
            !embedder().truncate,
            "OllamaEmbedder must not silently truncate oversized input by default"
        );
    }

    #[test]
    fn truncation_is_opt_in_and_reaches_the_wire() {
        // Both request shapes must carry the setting — the single-text path and
        // the batch path were separate literals, and only fixing one would leave
        // ingestion (which uses the batch path) still truncating silently.
        let strict = embedder();
        let lax = embedder().with_truncation(true);

        let strict_single = serde_json::to_value(EmbedRequest {
            model: &strict.model,
            input: "x",
            truncate: strict.truncate,
            options: None,
        })
        .unwrap();
        let texts = vec!["x".to_string()];
        let lax_batch = serde_json::to_value(EmbedBatchRequest {
            model: &lax.model,
            input: &texts,
            truncate: lax.truncate,
            options: None,
        })
        .unwrap();

        assert_eq!(strict_single["truncate"], serde_json::json!(false));
        assert_eq!(lax_batch["truncate"], serde_json::json!(true));
    }
}
