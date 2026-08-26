use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use vecdb_common::{FileType, FileTypeDetector};
use vecdb_core::chunking::{ChunkParams, Chunker, FixedWidthChunker};
use vecdb_core::ingestion::IngestionOptions;

struct MockBackend;
#[async_trait::async_trait]
impl vecdb_core::backend::Backend for MockBackend {
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

struct MockFactory;
impl vecdb_core::parsers::ParserFactory for MockFactory {
    fn get_parser(&self, _file_type: FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        None
    }
}

struct UnknownDetector;
impl FileTypeDetector for UnknownDetector {
    fn detect(&self, _path: &Path, _content: &[u8]) -> FileType {
        FileType::Unknown
    }
}

struct RealDetector;
impl FileTypeDetector for RealDetector {
    fn detect(&self, path: &Path, _content: &[u8]) -> FileType {
        FileType::from_path(path)
    }
}

struct TextBypassFactory;
impl vecdb_core::parsers::ParserFactory for TextBypassFactory {
    fn get_parser(&self, file_type: FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
        match file_type {
            FileType::Text => None,
            _ => None,
        }
    }
}

fn generate_large_lua_like_code(size_mb: usize) -> String {
    let line = "local function data_processor(arg1, arg2)\n    if arg1 ~= nil then\n        return arg2 * 2\n    end\n    print('error')\nend\n";
    let target_len = size_mb * 1024 * 1024;
    line.repeat(target_len / line.len() + 1)
}

/// See the identical helper in `perf_ingestion.rs`: wall-clock assertions are
/// enforced only under `VECDB_PERF_ASSERT=1` (`make test-perf`, serial), because
/// the gate runs test binaries concurrently and an absolute duration measured
/// there reports machine load as much as it reports a regression.
fn perf_assertions_enabled() -> bool {
    std::env::var("VECDB_PERF_ASSERT").as_deref() == Ok("1")
}

#[tokio::test]
async fn regression_lua_speed_and_structure() {
    // 1. PERFORMANCE CHECK
    let lua_content = generate_large_lua_like_code(5); // 5MB
    let tmp = TempDir::new().unwrap();
    let file_path = tmp.path().join("large.lua");
    File::create(&file_path)
        .unwrap()
        .write_all(lua_content.as_bytes())
        .unwrap();

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    // CRITICAL: UnknownDetector forces the "FileType::Unknown" path in ingestion.rs
    let detector: Arc<dyn FileTypeDetector> = Arc::new(UnknownDetector);
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(MockFactory);

    let options = IngestionOptions {
        path: tmp.path().to_str().unwrap().to_string(),
        collection: "regress_lua".to_string(),
        target_chunk_size: 1000,
        max_chunk_bytes: Some(2000),
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 0,
        respect_gitignore: false,
        ignore_vectorignore: false,
        vecdbrc_routes: None,
        vecdbrc_root: None,
        strategy: "recursive".to_string(), // Requesting recursive, but Unknown type should override to Simple
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
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    };

    let start = Instant::now();
    vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options, None)
        .await
        .unwrap();
    let duration = start.elapsed();

    println!("5MB Lua ingestion took: {:?}", duration);

    // ASSERT: Speed must be fast (FixedWidthChunker speed), not slow (RecursiveChunker
    // speed). Recursive chunking of this input runs ~30s, so the fallback this
    // guards against is an order-of-magnitude event, not a few hundred ms.
    //
    // The old threshold was 2s on the premise that the fast path takes ~15ms.
    // It does not: measured here it is ~2.3s, so the margin the number assumed
    // never existed and the test failed under the suite's parallel load while
    // passing alone. Raised to 10s, which still separates 2.3s from 30s, and
    // enforced only when timing is what is being tested — see
    // `perf_assertions_enabled` and `make test-perf`.
    //
    // The structural assertions below are the load-independent half of this
    // test and always run: they verify FixedWidthChunker actually split on line
    // boundaries, which is the behaviour the timing was standing in for.
    assert!(
        !perf_assertions_enabled() || duration < Duration::from_secs(10),
        "Performance Regression: Lua ingestion took too long ({:?}). It likely fell back to Recursive chunking.",
        duration
    );

    // 2. STRUCTURE CHECK (Did we actually use Line chunking?)
    // We can't easily capture the chunks from ingest_path without mocking Backend to capture store.
    // So we manually use the FixedWidthChunker here and verify the logic replicates what we expect.

    let chunker = FixedWidthChunker;
    let params = ChunkParams {
        target_chunk_size: 100,
        max_chunk_bytes: Some(200),
        chunk_overlap: 0,
        tokenizer: "bytes".to_string(),
        file_extension: None,
    };

    let code_snippet = "line1\nline2\nline3\nline4\nline5\n"; // 30 bytes
    let chunks = chunker.chunk(code_snippet, &params).await.unwrap();

    // Simple/Line chunker should preserve newlines and structure
    // With target_chunk_size 100, it should fit entirely or be split by lines if small.
    // Actually FixedWidthChunker aggregates lines until target_chunk_size.
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, "line1\nline2\nline3\nline4\nline5\n");

    // Now test splitting
    let params_small = ChunkParams {
        target_chunk_size: 12,
        max_chunk_bytes: Some(12), // CRITICAL FIX: FixedWidthChunker only cares about this
        chunk_overlap: 0,
        tokenizer: "bytes".to_string(),
        file_extension: None,
    };
    let chunks_split = chunker.chunk(code_snippet, &params_small).await.unwrap();

    // "line1\n" is 6 chars.
    // "line1\nline2\n" is 12 chars.
    // It should split roughly every 2 lines.
    assert!(
        chunks_split.len() > 1,
        "FixedWidthChunker failed to split content. Chunks: {}",
        chunks_split.len()
    );
    for chunk in chunks_split {
        assert!(
            chunk.content.ends_with('\n'),
            "FixedWidthChunker failed to preserve line boundary: {:?}",
            chunk.content
        );
    }
}

#[tokio::test]
async fn regression_text_performance() {
    // 3. TEXT PERFORMANCE CHECK (Simulate Pride and Prejudice)
    // We want to ensure RecursiveChunker doesn't choke on standard prose.
    // P&P is ~700KB. Let's do 5MB of dense prose.
    let paragraph = "It is a truth universally acknowledged, that a single man in possession of a good fortune, must be in want of a wife. However little known the feelings or views of such a man may be on his first entering a neighbourhood, this truth is so well fixed in the minds of the surrounding families, that he is considered the rightful property of some one or other of their daughters.\n";
    let target_len = 5 * 1024 * 1024;
    let prose_content = paragraph.repeat(target_len / paragraph.len() + 1);

    let tmp = TempDir::new().unwrap();
    let file_path = tmp.path().join("pride_sim.txt");
    File::create(&file_path)
        .unwrap()
        .write_all(prose_content.as_bytes())
        .unwrap();

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    // Real detector for .txt gives FileType::Text

    let detector: Arc<dyn FileTypeDetector> = Arc::new(RealDetector);
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(TextBypassFactory);

    let options = IngestionOptions {
        path: tmp.path().to_str().unwrap().to_string(),
        collection: "regress_text".to_string(),
        target_chunk_size: 1000,
        max_chunk_bytes: Some(2000),
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 0,
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
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    };

    println!("Starting Text Regression (Recursive/Smart)...");
    let start = Instant::now();
    vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options, None)
        .await
        .unwrap();
    let duration = start.elapsed();

    println!("5MB Text ingestion took: {:?}", duration);
    // Gated like every other wall-clock assertion here — see
    // `perf_assertions_enabled`. Measured under `cargo test`'s default
    // parallelism this competes with the other ingestion benchmarks in this
    // binary and with other test binaries, so it measures machine load as much
    // as the code. `make test-perf` runs it serially, where it means something.
    assert!(
        !perf_assertions_enabled() || duration.as_secs() < 10,
        "Performance Regression: Text ingestion took too long ({:?})",
        duration
    );
}
#[tokio::test]
async fn regression_pride_and_prejudice_file() {
    // 4. REAL FILE REGRESSION (Pride and Prejudice)
    //
    // Resolved from the crate, not from an absolute path. The absolute form
    // meant this skipped unconditionally on every machine but one — a test
    // that cannot fail is not a regression test, and it read as "passing".
    let file_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/external/pride-and-prejudice.txt");
    let file_path = file_path.as_path();

    // Only run if file exists (it's external, dependent on init.sh)
    if !file_path.exists() {
        println!("Skipping real P&P test: file not found");
        return;
    }

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);

    let detector: Arc<dyn FileTypeDetector> = Arc::new(RealDetector);
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(TextBypassFactory);

    let options = IngestionOptions {
        path: file_path.to_str().unwrap().to_string(),
        collection: "regress_pp".to_string(),
        target_chunk_size: 1000,
        max_chunk_bytes: Some(2000),
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 0,
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
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    };

    println!("Starting Real P&P Regression...");
    let start = Instant::now();
    vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options, None)
        .await
        .unwrap();
    let duration = start.elapsed();

    println!("Real P&P ingestion took: {:?}", duration);
    // Gated: see the note on the 5MB text case above. This one is the reason —
    // 735KB ingests in ~0.75s alone but was observed at 4.17s inside the full
    // suite, failing a 3s threshold for reasons that had nothing to do with the
    // ingestion path.
    assert!(
        !perf_assertions_enabled() || duration.as_secs() < 3,
        "Performance Regression: P&P took too long ({:?})",
        duration
    );
}
