use std::path::Path;
use anyhow::{Context, Result};
use reqwest::{Client, Url};
use serde::Deserialize;


pub struct SnapshotManager {
    client: Client,
    base_url: Url,
}

#[derive(Deserialize, Debug)]
struct SnapshotDescription {
    name: String,
    // creation_time: Option<String>,
    // size: u64,
}

#[derive(Deserialize, Debug)]
struct ListResponse {
    result: Vec<SnapshotDescription>,
    // status: String,
}

#[derive(Deserialize, Debug)]
struct CreateResponse {
    result: SnapshotDescription,
}

impl SnapshotManager {
    pub fn new(qdrant_url: &str) -> Result<Self> {
        let mut url = Url::parse(qdrant_url).context("Invalid Qdrant URL")?;
        
        // Heuristic: If port is 6334 (default gRPC), switch to 6333 (default HTTP)
        if let Some(port) = url.port() {
            if port == 6334 {
                url.set_port(Some(6333)).map_err(|_| anyhow::anyhow!("Failed to set port"))?;
            }
        }

        Ok(Self {
            client: Client::new(),
            base_url: url,
        })
    }

    pub async fn list(&self, collection: &str) -> Result<Vec<String>> {
        let url = self.base_url.join(&format!("collections/{}/snapshots", collection))?;
        let resp = self.client.get(url).send().await?;
        
        if !resp.status().is_success() {
             return Err(anyhow::anyhow!("Qdrant Error: {}", resp.status()));
        }

        let body: ListResponse = resp.json().await?;
        Ok(body.result.into_iter().map(|s| s.name).collect())
    }

    pub async fn create(&self, collection: &str) -> Result<String> {
        let url = self.base_url.join(&format!("collections/{}/snapshots", collection))?;
        let resp = self.client.post(url).send().await?;
        
        if !resp.status().is_success() {
             return Err(anyhow::anyhow!("Failed to create snapshot: {}", resp.status()));
        }
        
        let body: CreateResponse = resp.json().await?;
        Ok(body.result.name)
    }

    pub async fn download(&self, collection: &str, snapshot_name: &str, output_path: &Path) -> Result<()> {
        let url = self.base_url.join(&format!("collections/{}/snapshots/{}", collection, snapshot_name))?;
        let mut resp = self.client.get(url).send().await?;
        
        if !resp.status().is_success() {
             return Err(anyhow::anyhow!("Failed to download snapshot: {}", resp.status()));
        }

        let mut file = tokio::fs::File::create(output_path).await?;
        while let Some(chunk) = resp.chunk().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        }
        
        Ok(())
    }

    pub async fn restore(&self, collection: &str, file_path: &Path) -> Result<()> {
        // 1. Check if collection exists, if not create it (optional, logic might be complex)
        // Actually, Qdrant allows snapshot upload to recover a collection.
        // Endpoint: POST /collections/{name}/snapshots/upload
        
        let url = self.base_url.join(&format!("collections/{}/snapshots/upload", collection))?;
        
        // Read file
        let file_bytes = tokio::fs::read(file_path).await?;
        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_path.file_name().unwrap_or_default().to_string_lossy().to_string());
            
        let form = reqwest::multipart::Form::new().part("snapshot", part);

        let resp = self.client.post(url)
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
             let text = resp.text().await?;
             return Err(anyhow::anyhow!("Failed to restore snapshot: {} - {}", text, "Check if collection exists or try creating it first."));
        }
        
        // After upload, we might need to "recover" it? 
        // Docs say: "Snapshot will be uploaded and recovered automatically" for this endpoint usually?
        // Wait, looking at Qdrant docs:
        // POST /collections/{name}/snapshots/upload
        // "Upload snapshot from a file and recover it"
        // YES.
        
        Ok(())
    }
}
