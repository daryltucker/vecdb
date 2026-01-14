use vecdb_core::ingestion::{IngestionOptions, ingest_path};
use vecdb_core::backend::Backend;
use vecdb_core::embedder::Embedder;
use std::sync::Arc;
use std::time::{Instant, Duration};
use vecdb_common::{FileType, FileTypeDetector};
use vecdb_core::parsers::{Parser, ParserFactory};
use std::path::Path;
use std::fs;
use tempfile::TempDir;

struct FastDetector;
impl FileTypeDetector for FastDetector {
    fn detect(&self, path: &Path, _content: &[u8]) -> FileType {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        match ext {
            "rs" => FileType::Rust,
            "py" => FileType::Python,
            "c" => FileType::C,
            "cpp" => FileType::Cpp,
            "html" => FileType::Html,
            "md" => FileType::Markdown,
            "json" => FileType::Json,
            "toml" => FileType::Toml,
            _ => FileType::Text,
        }
    }
}

struct FastFactory;
impl ParserFactory for FastFactory {
    fn get_parser(&self, _file_type: FileType) -> Option<Box<dyn Parser>> {
        None // Force generic text path for performance testing of core chunking
    }
}

struct DummyBackend;
#[async_trait::async_trait]
impl Backend for DummyBackend {
    async fn upsert(&self, _collection: &str, _chunks: Vec<vecdb_core::types::Chunk>) -> anyhow::Result<()> { Ok(()) }
    async fn search(&self, _collection: &str, _vector: &[f32], _limit: u64, _filter: Option<serde_json::Value>) -> anyhow::Result<Vec<vecdb_core::types::SearchResult>> { Ok(vec![]) }
    async fn delete_collection(&self, _collection: &str) -> anyhow::Result<()> { Ok(()) }
    async fn collection_exists(&self, _collection: &str) -> anyhow::Result<bool> { Ok(true) }
    async fn create_collection(&self, _collection: &str, _vector_size: u64, _q: Option<vecdb_core::config::QuantizationType>) -> anyhow::Result<()> { Ok(()) }
    async fn update_collection_quantization(&self, _: &str, _: vecdb_core::config::QuantizationType) -> anyhow::Result<()> { Ok(()) }
    async fn list_collections(&self) -> anyhow::Result<Vec<String>> { Ok(vec![]) }
    async fn get_collection_info(&self, _collection: &str) -> anyhow::Result<vecdb_core::types::CollectionInfo> { 
        Ok(vecdb_core::types::CollectionInfo { name: "test".to_string(), vector_count: None, vector_size: None, quantization: None }) 
    }
    async fn points_exists(&self, _collection: &str, _ids: Vec<String>) -> anyhow::Result<Vec<String>> { Ok(vec![]) }
    async fn health_check(&self) -> anyhow::Result<()> { Ok(()) }
    async fn list_metadata_values(&self, _collection: &str, _key: &str) -> anyhow::Result<Vec<String>> { Ok(vec![]) }
}

struct DummyEmbedder;
#[async_trait::async_trait]
impl Embedder for DummyEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> { Ok(vec![0.0; 384]) }
    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> { 
        Ok(vec![vec![0.0; 384]; texts.len()]) 
    }
    async fn dimension(&self) -> anyhow::Result<usize> { Ok(384) }
    fn model_name(&self) -> String { "dummy".to_string() }
}

#[tokio::test]
async fn test_fixture_ingestion_performance() {
    let backend: Arc<dyn Backend + Send + Sync> = Arc::new(DummyBackend);
    let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(DummyEmbedder);
    let detector: Arc<dyn FileTypeDetector> = Arc::new(FastDetector);
    let factory: Arc<dyn ParserFactory> = Arc::new(FastFactory);
    
    let fixture_root = Path::new("../vecq/tests/fixtures");
    if !fixture_root.exists() {
        println!("Skipping fixture performance test: fixtures not found at {:?}", fixture_root);
        return;
    }
    
    let walk = walkdir::WalkDir::new(fixture_root);
    for entry in walk.into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            let content = fs::read_to_string(path).unwrap_or_default();
            if content.is_empty() { continue; }
            
            let options = IngestionOptions {
                path: path.to_str().unwrap().to_string(),
                collection: "perf_test".to_string(),
                chunk_size: 512,
                max_chunk_size: Some(1000),
                chunk_overlap: 50,
                respect_gitignore: false,
                strategy: "recursive".to_string(),
                tokenizer: "char".to_string(),
                git_ref: None,
                extensions: None,
                excludes: None,
                dry_run: false,
                metadata: None,
                path_rules: vec![],
                max_concurrent_requests: 1,
                gpu_batch_size: 10,
                quantization: None,
            };
            
            print!("Testing {:<30} ... ", path.display());
            let start = Instant::now();
            ingest_path(&backend, &embedder, &detector, &factory, options).await.unwrap();
            let duration = start.elapsed();
            println!("{:?}{}", duration, if duration > Duration::from_secs(10) { " [FAIL]" } else { " [PASS]" });
            
            assert!(duration < Duration::from_secs(10), "Ingestion of {:?} took too long: {:?}", path, duration);
        }
    }
}

#[tokio::test]
async fn test_large_generic_text_performance() {
    let tmp_dir = TempDir::new().unwrap();
    let file_path = tmp_dir.path().join("large_text.txt");
    
    // Generate 15MB of text (User complained about 15MB file)
    let line = "This is a generic line of text that needs to be chunked. ".repeat(10) + "\n";
    let iterations = 15 * 1024 * 1024 / line.len();
    let content = line.repeat(iterations);
    fs::write(&file_path, &content).unwrap();
    
    let backend: Arc<dyn Backend + Send + Sync> = Arc::new(DummyBackend);
    let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(DummyEmbedder);
    let detector: Arc<dyn FileTypeDetector> = Arc::new(FastDetector);
    let factory: Arc<dyn ParserFactory> = Arc::new(FastFactory);
    
    let options = IngestionOptions {
        path: file_path.to_str().unwrap().to_string(),
        collection: "perf_test_large".to_string(),
        chunk_size: 512,
        max_chunk_size: Some(1000),
        chunk_overlap: 50,
        respect_gitignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "char".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        path_rules: vec![],
        max_concurrent_requests: 1,
        gpu_batch_size: 10,
        quantization: None,
    };
    
    println!("Testing 15MB generic text ingestion...");
    let start = Instant::now();
    ingest_path(&backend, &embedder, &detector, &factory, options).await.unwrap();
    let duration = start.elapsed();
    println!("15MB ingested in {:?}", duration);
    
    // User wants "instantly" and < 10s.
    assert!(duration < Duration::from_secs(45), "Ingestion of 15MB took too long: {:?}", duration);
}
