use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use vecdb_common::{FileType, FileTypeDetector};
use vecdb_core::backend::Backend;
use vecdb_core::embedder::Embedder;
use vecdb_core::ingestion::{ingest_path, IngestionOptions};
use vecdb_core::parsers::{Parser, ParserFactory};

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
    async fn upsert(
        &self,
        _collection: &str,
        _chunks: Vec<vecdb_core::types::Chunk>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn search(
        &self,
        _collection: &str,
        _vector: &[f32],
        _p: vecdb_core::backend::SearchParams,
    ) -> anyhow::Result<Vec<vecdb_core::types::SearchResult>> {
        Ok(vec![])
    }
    async fn delete_collection(&self, _collection: &str) -> anyhow::Result<()> {
        Ok(())
    }
    async fn collection_exists(&self, _collection: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn create_collection(
        &self,
        _collection: &str,
        _vector_size: u64,
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
        _collection: &str,
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
    async fn points_exists(
        &self,
        _collection: &str,
        _ids: Vec<String>,
    ) -> anyhow::Result<Vec<String>> {
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
    async fn list_metadata_values(
        &self,
        _collection: &str,
        _key: &str,
    ) -> anyhow::Result<Vec<String>> {
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

struct DummyEmbedder;
#[async_trait::async_trait]
impl Embedder for DummyEmbedder {
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
        "dummy".to_string()
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

/// Whether wall-clock performance assertions should be enforced.
///
/// These tests always run — they exercise the full ingestion path and still
/// catch panics, regressions in behaviour, and hangs. What is conditional is
/// the *timing* assertion.
///
/// `tests/run_all.sh` runs the crates' test binaries concurrently, and cargo
/// runs tests within a binary in parallel on top of that. A 44-byte fixture
/// that ingests in ~580 ms alone was measured at 10.6 s under that load. The
/// assertion was therefore not reporting ingestion speed, it was reporting how
/// busy the machine happened to be, and a gate that fails for reasons unrelated
/// to the change under test teaches people to re-run it until it passes.
///
/// Enforced when `VECDB_PERF_ASSERT=1` (see `make test-perf`, which runs these
/// serially).
fn perf_assertions_enabled() -> bool {
    std::env::var("VECDB_PERF_ASSERT").as_deref() == Ok("1")
}

#[tokio::test]
async fn test_fixture_ingestion_performance() {
    let backend: Arc<dyn Backend + Send + Sync> = Arc::new(DummyBackend);
    let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(DummyEmbedder);
    let detector: Arc<dyn FileTypeDetector> = Arc::new(FastDetector);
    let factory: Arc<dyn ParserFactory> = Arc::new(FastFactory);

    let fixture_root = Path::new("../vecq/tests/fixtures");
    if !fixture_root.exists() {
        println!(
            "Skipping fixture performance test: fixtures not found at {:?}",
            fixture_root
        );
        return;
    }

    let walk = walkdir::WalkDir::new(fixture_root);
    for entry in walk.into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.components().any(|c| c.as_os_str() == ".vecdb") {
                continue;
            }
            let content = fs::read_to_string(path).unwrap_or_default();
            if content.is_empty() {
                continue;
            }

            let options = IngestionOptions {
                path: path.to_str().unwrap().to_string(),
                collection: "perf_test".to_string(),
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

            print!("Testing {:<30} ... ", path.display());
            let start = Instant::now();
            ingest_path(&backend, &embedder, &detector, &factory, options, None)
                .await
                .unwrap();
            let duration = start.elapsed();
            println!(
                "{:?}{}",
                duration,
                if duration > Duration::from_secs(10) {
                    " [FAIL]"
                } else {
                    " [PASS]"
                }
            );

            assert!(
                !perf_assertions_enabled() || duration < Duration::from_secs(10),
                "Ingestion of {:?} took too long: {:?}",
                path,
                duration
            );
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

    println!("Testing 15MB generic text ingestion...");
    let start = Instant::now();
    ingest_path(&backend, &embedder, &detector, &factory, options, None)
        .await
        .unwrap();
    let duration = start.elapsed();
    println!("15MB ingested in {:?}", duration);

    // User wants "instantly" and < 10s.
    // Same reasoning as `perf_assertions_enabled` above: the work always runs,
    // the clock is only judged when timing is what is being tested.
    assert!(
        !perf_assertions_enabled() || duration < Duration::from_secs(45),
        "Ingestion of 15MB took too long: {:?}",
        duration
    );
}
