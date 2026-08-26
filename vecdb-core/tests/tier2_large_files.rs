use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use vecdb_common::{FileType, FileTypeDetector};
use vecdb_core::ingestion::IngestionOptions;

// --- MOCKS ---

struct MockBackend;
#[async_trait::async_trait]
impl vecdb_core::backend::Backend for MockBackend {
    async fn upsert(
        &self,
        _collection: &str,
        chunks: Vec<vecdb_core::types::Chunk>,
    ) -> anyhow::Result<()> {
        // Just verify we got chunks
        if chunks.is_empty() {
            return Err(anyhow::anyhow!("MockBackend received empty chunks"));
        }
        Ok(())
    }
    // Stubs
    async fn search(
        &self,
        _: &str,
        _: &[f32],
        _p: vecdb_core::backend::SearchParams,
    ) -> anyhow::Result<Vec<vecdb_core::types::SearchResult>> {
        Ok(vec![])
    }
    async fn delete_collection(&self, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn collection_exists(&self, _: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn create_collection(
        &self,
        _name: &str,
        _v: u64,
        _q: Option<vecdb_core::config::QuantizationType>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn update_collection_quantization(
        &self,
        _: &str,
        _: vecdb_core::config::QuantizationType,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list_collections(&self) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn get_collection_info(
        &self,
        _: &str,
    ) -> anyhow::Result<vecdb_core::types::CollectionInfo> {
        Ok(vecdb_core::types::CollectionInfo {
            name: "test".to_string(),
            vector_count: None,
            vector_size: None,
            quantization: None,
            vectors_on_disk: None,
            payload_on_disk: None,
        })
    }
    async fn points_exists(&self, _: &str, _: Vec<String>) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }

    async fn delete_stale_points(
        &self,
        _c: &str,
        _d: &str,
        _k: &[String],
    ) -> anyhow::Result<usize> {
        Ok(0)
    }
    async fn health_check(&self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list_metadata_values(&self, _: &str, _: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn get_collection_id(&self, _collection: &str) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn set_collection_id(&self, _collection: &str, _id: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn list_tasks(&self) -> anyhow::Result<Vec<vecdb_core::types::TaskInfo>> {
        Ok(vec![])
    }

    async fn write_genesis(
        &self,
        _c: &str,
        _m: &vecdb_core::types::GenesisMetadata,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn read_genesis(&self, _c: &str) -> anyhow::Result<vecdb_core::types::CollectionGenesis> {
        // Mirror MockEmbedder::identity so the space guard sees a matching
        // contract. `present: false` would (correctly) make every ingest here
        // fail as "not created by vecdb".
        Ok(vecdb_core::types::CollectionGenesis {
            collection_id: Some("mock-collection".to_string()),
            model: vecdb_core::types::ModelIdentity {
                name: "mock-embedder".to_string(),
                digest: Some("mock:test-double".to_string()),
                architecture: Some("mock".to_string()),
                family: Some("mock".to_string()),
                parameter_size: Some("0".to_string()),
                quantization_level: Some("none".to_string()),
                embedding_length: None,
                context_length: Some(8192),
            },
            dimension: None,
            distance: Some("Cosine".to_string()),
            created_at: None,
            vecdb_version: Some("test".to_string()),
            vecdb_revision: None,
            chunking: None,
        })
    }
}

struct MockEmbedder;
#[async_trait::async_trait]
impl vecdb_core::embedder::Embedder for MockEmbedder {
    async fn embed(&self, _text: &str, target_dim: Option<usize>) -> anyhow::Result<Vec<f32>> {
        let dim = target_dim.unwrap_or(384);
        Ok(vec![0.1; dim])
    }
    async fn embed_batch(
        &self,
        texts: &[String],
        target_dim: Option<usize>,
    ) -> anyhow::Result<Vec<Vec<f32>>> {
        let dim = target_dim.unwrap_or(384);
        Ok(vec![vec![0.1; dim]; texts.len()])
    }
    async fn dimension(&self) -> anyhow::Result<usize> {
        Ok(384)
    }
    fn model_name(&self) -> String {
        "mock".to_string()
    }

    async fn identity(&self) -> anyhow::Result<vecdb_core::types::ModelIdentity> {
        // Shared sentinel: every test double is one embedding space, so the
        // compatibility guard passes and these tests exercise what they are
        // actually about. Guard behaviour has its own dedicated tests.
        Ok(vecdb_core::types::ModelIdentity {
            name: "mock-embedder".to_string(),
            digest: Some("mock:test-double".to_string()),
            architecture: Some("mock".to_string()),
            family: Some("mock".to_string()),
            parameter_size: Some("0".to_string()),
            quantization_level: Some("none".to_string()),
            embedding_length: None,
            context_length: Some(8192),
        })
    }
}

struct RealDetector;
impl FileTypeDetector for RealDetector {
    fn detect(&self, path: &Path, _content: &[u8]) -> FileType {
        FileType::from_path(path)
    }
}

// Pass-through factory that lets `vecdb_core` decide to use built-ins or Code/Recursive
struct PassThroughFactory;
impl vecdb_core::parsers::ParserFactory for PassThroughFactory {
    fn get_parser(&self, _file_type: FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        // Return None to force the core to use its default Chunker logic (Recursive/Code/TwoPass)
        // unless it's a type that strictly REQUIRES a parser.
        None
    }
    fn get_streaming_parser(
        &self,
        file_type: FileType,
    ) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        match file_type {
            FileType::Json => Some(Box::new(
                vecdb_core::parsers::streaming_json::StreamingJsonParser::new(),
            )),
            _ => None,
        }
    }
}

// --- HELPER ---

fn generate_large_file(path: &Path, size_mb: usize, pattern: &str) {
    let mut f = File::create(path).unwrap();
    let target_size = size_mb * 1024 * 1024;
    let mut current_size = 0;
    while current_size < target_size {
        f.write_all(pattern.as_bytes()).unwrap();
        current_size += pattern.len();
    }
}

// --- TESTS ---

#[tokio::test]
async fn test_large_file_bifurcation_ast() {
    // 60MB Rust file -> Should trigger Two-Pass ingestion
    // because it is > 50MB and NOT supported by streaming parser (only JSON is).

    let tmp = TempDir::new().unwrap();
    let file_path = tmp.path().join("giant.rs");

    // Generate valid-ish Rust code so it doesn't crash a real parser if one were used
    // THIS TEST IS A CHECK OF THE VALIDITY OF YOUR CODE
    // THIS TEST IS NOT TO BE ADJUSTED TO MAKE IT EASIER TO PASS
    // DO NOT CHANGE THE FILESIZE.  DO NOT CHANGE THE TIMING
    // IF THIS TEST FAILS, YOUR CODE HAS FAILED; FIX YOUR CODE!
    let pattern = "fn function_name() { println!(\"hello\"); }\n";
    generate_large_file(&file_path, 60, pattern);

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    let detector: Arc<dyn FileTypeDetector> = Arc::new(RealDetector);
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(PassThroughFactory);

    let options = IngestionOptions {
        path: file_path.to_str().unwrap().to_string(),
        collection: "test_large_ast".to_string(),
        target_chunk_size: 512,
        max_chunk_bytes: Some(1000),
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 50,
        respect_gitignore: false,
        ignore_vectorignore: false,
        vecdbrc_routes: None,
        vecdbrc_root: None,
        strategy: "code_aware".to_string(),
        tokenizer: "bytes".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        file_allowlist: None,
        project_root: None,
        metadata: None,
        path_rules: vec![],
        max_concurrent_requests: 1,
        gpu_batch_size: 10,
        quantization: None,
        allow_quantization_delta: false,
    };

    println!("Ingesting 60MB Rust file (AST/Code Path)...");
    let start = Instant::now();
    let result =
        vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options, None)
            .await;
    let duration = start.elapsed();

    assert!(result.is_ok(), "Ingestion failed: {:?}", result.err());
    println!("60MB AST Ingestion took: {:?}", duration);

    // Hard to inspect internal state (Did it use TwoPass?), but success implies it handled it.
    // If it tried to load all 60MB into a single string for `CodeChunker`, it might have spiked RAM,
    // but TwoPass reads in 5MB segments.
}

#[tokio::test]
async fn test_large_file_streaming_json() {
    // 60MB JSON file -> Should trigger Streaming Parser

    let tmp = TempDir::new().unwrap();
    let file_path = tmp.path().join("giant.json");

    // Generate a massive JSON array
    let mut f = File::create(&file_path).unwrap();
    f.write_all(b"[").unwrap();
    let item = r#"{"id": 1, "content": "some content"},"#;
    let target = 60 * 1024 * 1024;
    let mut current = 1;
    while current < target {
        f.write_all(item.as_bytes()).unwrap();
        current += item.len();
    }
    // Close array (hacky, trailing comma might be issue for loose parsers, strict JSON disallows)
    // Let's make it valid: replace last comma with ]
    // Or just write {} to end it
    f.write_all(b"{\"id\": 999, \"content\": \"last\"}]")
        .unwrap();

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    let detector: Arc<dyn FileTypeDetector> = Arc::new(RealDetector);
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(PassThroughFactory);

    let options = IngestionOptions {
        path: file_path.to_str().unwrap().to_string(),
        collection: "test_large_json".to_string(),
        target_chunk_size: 512,
        max_chunk_bytes: Some(1000),
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 50,
        respect_gitignore: false,
        ignore_vectorignore: false,
        vecdbrc_routes: None,
        vecdbrc_root: None,
        strategy: "recursive".to_string(),
        tokenizer: "bytes".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        file_allowlist: None,
        project_root: None,
        metadata: None,
        path_rules: vec![],
        max_concurrent_requests: 1,
        gpu_batch_size: 10,
        quantization: None,
        allow_quantization_delta: false,
    };

    println!("Ingesting 60MB JSON file (Streaming Path)...");
    let start = Instant::now();
    let result =
        vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options, None)
            .await;
    let duration = start.elapsed();

    assert!(result.is_ok(), "Ingestion failed: {:?}", result.err());
    println!("60MB Streaming Ingestion took: {:?}", duration);
}
