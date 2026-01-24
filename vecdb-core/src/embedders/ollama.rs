/*
 * PURPOSE:
 *   Concrete implementation of Embedder trait using Ollama API.
 *   Provides easy local embedding generation.
 *
 * REQUIREMENTS:
 *   - Connect to local/remote Ollama instance
 *   - Support configured model (default: nomic-embed-text)
 *   - Handle API errors gracefully
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
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    embedding: Vec<f32>,
}

impl OllamaEmbedder {
    pub fn new(
        base_url: String,
        model: String,
        accept_invalid_certs: bool,
        api_key: Option<String>,
    ) -> Self {
        let mut builder =
            reqwest::ClientBuilder::new().danger_accept_invalid_certs(accept_invalid_certs);

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
        }
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let url = format!("{}/api/embeddings", self.base_url);

        let request = EmbeddingRequest {
            model: &self.model,
            prompt: text,
        };

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to connect to Ollama")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API error: {}", error_text);
        }

        let embedding_response: EmbeddingResponse = response
            .json()
            .await
            .context("Failed to parse Ollama response")?;

        Ok(embedding_response.embedding)
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        use futures::stream::{self, StreamExt};

        // Create owned data to avoid lifetime issues with async stream
        let text_list: Vec<String> = texts.to_vec();

        // Create a stream of futures, buffered to limit concurrency while PRESERVING ORDER
        let vectors = stream::iter(text_list)
            .map(|text| {
                let this = self.clone();
                async move { this.embed(&text).await }
            })
            .buffered(5) // Max 5 concurrent requests, ordered
            .collect::<Vec<_>>()
            .await;

        // Collect results, checking for errors
        let mut results = Vec::with_capacity(texts.len());
        for res in vectors {
            results.push(res?);
        }

        Ok(results)
    }

    async fn dimension(&self) -> Result<usize> {
        // Probe via a small embedding to get dimension
        let dummy = self.embed("probe").await?;
        Ok(dummy.len())
    }

    fn model_name(&self) -> String {
        format!("ollama:{}", self.model)
    }
}
