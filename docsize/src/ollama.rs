use anyhow::{Context, Result};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;

pub trait InferenceEngine {
    #[allow(dead_code)]
    async fn complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<String>;
    async fn stream_complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<BoxStream<'static, Result<String>>>;
}

pub struct OllamaEngine {
    pub url: String,
}

impl OllamaEngine {
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let client = reqwest::Client::new();
        let api_url = format!("{}/api/tags", self.url.trim_end_matches('/'));
        
        let resp = client.get(&api_url)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;
        let models = json.get("models")
            .and_then(|v| v.as_array())
            .context("Missing 'models' array in Ollama tags output")?;

        let mut names = Vec::new();
        for model in models {
            if let Some(name) = model.get("name").and_then(|v| v.as_str()) {
                names.push(name.to_string());
            }
        }
        Ok(names)
    }
}

impl InferenceEngine for OllamaEngine {
    async fn complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<String> {
        let client = reqwest::Client::new();
        let api_url = format!("{}/api/generate", self.url.trim_end_matches('/'));
        
        let mut payload = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false
        });

        if let Some(opts) = options {
            payload.as_object_mut().unwrap().insert("options".to_string(), opts);
        }

        let resp = client.post(&api_url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;
        let response = json.get("response")
            .and_then(|v| v.as_str())
            .context("Missing 'response' field in Ollama output")?;

        Ok(response.to_string())
    }

    async fn stream_complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<BoxStream<'static, Result<String>>> {
        let client = reqwest::Client::new();
        let api_url = format!("{}/api/generate", self.url.trim_end_matches('/'));
        
        let mut payload = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": true
        });

        if let Some(opts) = options {
            payload.as_object_mut().unwrap().insert("options".to_string(), opts);
        }

        let resp = client.post(&api_url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let stream = resp.bytes_stream().map(|item| {
            match item {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut combined = String::new();
                    for line in text.lines() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(token) = json.get("response").and_then(|v| v.as_str()) {
                                combined.push_str(token);
                            }
                        }
                    }
                    Ok(combined)
                },
                Err(e) => Err(anyhow::anyhow!(e))
            }
        });

        Ok(stream.boxed())
    }
}
