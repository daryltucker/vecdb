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
 *   - Set truncate=true to handle inputs exceeding model context window
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
    /// Auto-truncate input to fit model's context window.
    /// Without this, inputs exceeding the model's context length
    /// (e.g., 512 tokens for nomic-embed-text-v2-moe) cause hard errors.
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
        }
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
                       1. Reduce chunk_size in config.toml (e.g., chunk_size = 256)\n\
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
            truncate: true, // Use auto-truncate for safety
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
            truncate: true, // Use auto-truncate for safety
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
}

