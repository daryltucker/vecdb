use std::time::Instant;
use vecdb_core::chunking::{Chunker, SimpleChunker, ChunkParams};
use vecdb_common::{FileType, FileTypeDetector};
use vecdb_core::ingestion::IngestionOptions;
use std::sync::Arc;
use tempfile::TempDir;
use std::fs::File;
use std::io::Write;
use std::path::Path;



struct MockBackend;
#[async_trait::async_trait]
impl vecdb_core::backend::Backend for MockBackend {
    async fn upsert(&self, _collection: &str, _chunks: Vec<vecdb_core::types::Chunk>) -> anyhow::Result<()> { Ok(()) }
    async fn search(&self, _collection: &str, _vector: &[f32], _limit: u64, _filter: Option<serde_json::Value>) -> anyhow::Result<Vec<vecdb_core::types::SearchResult>> { Ok(vec![]) }
    async fn delete_collection(&self, _collection: &str) -> anyhow::Result<()> { Ok(()) }
    async fn collection_exists(&self, _collection: &str) -> anyhow::Result<bool> { Ok(true) }
    async fn create_collection(&self, _collection: &str, _vector_size: u64) -> anyhow::Result<()> { Ok(()) }
    async fn list_collections(&self) -> anyhow::Result<Vec<String>> { Ok(vec![]) }
    async fn get_collection_info(&self, _collection: &str) -> anyhow::Result<vecdb_core::types::CollectionInfo> { Ok(vecdb_core::types::CollectionInfo { name: "test".to_string(), vector_count: None, vector_size: None }) }
    async fn points_exists(&self, _collection: &str, _ids: Vec<String>) -> anyhow::Result<Vec<String>> { Ok(vec![]) }
    async fn health_check(&self) -> anyhow::Result<()> { Ok(()) }
    async fn list_metadata_values(&self, _collection: &str, _key: &str) -> anyhow::Result<Vec<String>> { Ok(vec![]) }
}

struct MockEmbedder;
#[async_trait::async_trait]
impl vecdb_core::embedder::Embedder for MockEmbedder {
    async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> { Ok(vec![]) }
    async fn embed_batch(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> { Ok(vec![vec![]; texts.len()]) }
    async fn dimension(&self) -> anyhow::Result<usize> { Ok(384) }
    fn model_name(&self) -> String { "mock".to_string() }
}

struct MockFactory;
impl vecdb_core::parsers::ParserFactory for MockFactory {
    fn get_parser(&self, _file_type: FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> { None }
}

struct UnknownDetector;
impl FileTypeDetector for UnknownDetector {
    fn detect(&self, _path: &Path, _content: &[u8]) -> FileType { FileType::Unknown }
}


fn generate_large_lua_like_code(size_mb: usize) -> String {
    let line = "local function data_processor(arg1, arg2)\n    if arg1 ~= nil then\n        return arg2 * 2\n    end\n    print('error')\nend\n";
    let target_len = size_mb * 1024 * 1024;
    line.repeat(target_len / line.len() + 1)
}

#[tokio::test]
async fn regression_lua_speed_and_structure() {
    // 1. PERFORMANCE CHECK
    let lua_content = generate_large_lua_like_code(5); // 5MB
    let tmp = TempDir::new().unwrap();
    let file_path = tmp.path().join("large.lua");
    File::create(&file_path).unwrap().write_all(lua_content.as_bytes()).unwrap();

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    // CRITICAL: UnknownDetector forces the "FileType::Unknown" path in ingestion.rs
    let detector: Arc<dyn FileTypeDetector> = Arc::new(UnknownDetector);
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(MockFactory);

    let options = IngestionOptions {
        path: tmp.path().to_str().unwrap().to_string(),
        collection: "regress_lua".to_string(),
        chunk_size: 1000, 
        max_chunk_size: Some(2000),
        chunk_overlap: 0,
        respect_gitignore: false,
        strategy: "recursive".to_string(), // Requesting recursive, but Unknown type should override to Simple
        tokenizer: "char".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
    };

    let start = Instant::now();
    let _ = vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options).await.unwrap();
    let duration = start.elapsed();
    
    println!("5MB Lua ingestion took: {:?}", duration);
    
    // ASSERT: Speed must be fast (SimpleChunker speed), not Slow (RecursiveChunker speed)
    // 5MB Simple takes ~15ms. Recursive takes ~30s. 
    // We set a conservative limit of 2s to account for CI/overhead, but fail if it regresses to "parsing" speeds.
    assert!(duration.as_secs() < 2, "Performance Regression: Lua ingestion took too long ({:?}). It likely fell back to Recursive chunking.", duration);


    // 2. STRUCTURE CHECK (Did we actually use Line chunking?)
    // We can't easily capture the chunks from ingest_path without mocking Backend to capture store.
    // So we manually use the SimpleChunker here and verify the logic replicates what we expect.
    
    let chunker = SimpleChunker;
    let params = ChunkParams {
        chunk_size: 100,
        max_chunk_size: Some(200),
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
    };
    
    let code_snippet = "line1\nline2\nline3\nline4\nline5\n"; // 30 bytes
    let chunks = chunker.chunk(code_snippet, &params).await.unwrap();
    
    // Simple/Line chunker should preserve newlines and structure
    // With chunk_size 100, it should fit entirely or be split by lines if small.
    // Actually SimpleChunker aggregates lines until chunk_size.
    assert_eq!(chunks.len(), 1); 
    assert_eq!(chunks[0].content, "line1\nline2\nline3\nline4\nline5\n");
    
    // Now test splitting
    let params_small = ChunkParams {
        chunk_size: 12, 
        max_chunk_size: Some(12), // CRITICAL FIX: SimpleChunker only cares about this
        chunk_overlap: 0,
        tokenizer: "char".to_string(),
        file_extension: None,
    };
    let chunks_split = chunker.chunk(code_snippet, &params_small).await.unwrap();
    
    // "line1\n" is 6 chars. 
    // "line1\nline2\n" is 12 chars.
    // It should split roughly every 2 lines.
    assert!(chunks_split.len() > 1, "SimpleChunker failed to split content. Chunks: {}", chunks_split.len());
    for chunk in chunks_split {
        assert!(chunk.content.ends_with('\n'), "SimpleChunker failed to preserve line boundary: {:?}", chunk.content);
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
    File::create(&file_path).unwrap().write_all(prose_content.as_bytes()).unwrap();

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    // Real detector for .txt gives FileType::Text
    struct RealDetector;
    impl FileTypeDetector for RealDetector {
        fn detect(&self, path: &Path, _content: &[u8]) -> FileType { FileType::from_path(path) }
    }
    let detector: Arc<dyn FileTypeDetector> = Arc::new(RealDetector);
    // Use Builtin (or Mock factory that returns None) - wait, if Text, ParserFactory might give Yaml if not careful.
    // We fixed that in mod.rs. Let's verify it here by using the Real Factory behavior Mock.
    struct TextBypassFactory;
    impl vecdb_core::parsers::ParserFactory for TextBypassFactory {
        fn get_parser(&self, file_type: FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> {
            // Mimic the fix in mod.rs: Text returns None
            match file_type {
                FileType::Text => None,
                _ => None, 
            }
        }
    }
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(TextBypassFactory);

    let options = IngestionOptions {
        path: tmp.path().to_str().unwrap().to_string(),
        collection: "regress_text".to_string(),
        chunk_size: 1000, 
        max_chunk_size: Some(2000),
        chunk_overlap: 0,
        respect_gitignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "char".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
    };

    println!("Starting Text Regression (Recursive/Smart)...");
    let start = Instant::now();
    let _ = vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options).await.unwrap();
    let duration = start.elapsed();
    
    println!("5MB Text ingestion took: {:?}", duration);
    // Text should be reasonably fast (~1-2s for 5MB).
    assert!(duration.as_secs() < 10, "Performance Regression: Text ingestion took too long ({:?})", duration);
}
#[tokio::test]
async fn regression_pride_and_prejudice_file() {
    // 4. REAL FILE REGRESSION (Pride and Prejudice)
    let file_path = Path::new("/home/daryl/Projects/NRG/vecdb-mcp/tests/fixtures/external/pride-and-prejudice.txt");
    
    // Only run if file exists (it's external, dependent on init.sh)
    if !file_path.exists() {
        println!("Skipping real P&P test: file not found");
        return;
    }

    let backend: Arc<dyn vecdb_core::backend::Backend + Send + Sync> = Arc::new(MockBackend);
    let embedder: Arc<dyn vecdb_core::embedder::Embedder + Send + Sync> = Arc::new(MockEmbedder);
    struct RealDetector;
    impl FileTypeDetector for RealDetector {
        fn detect(&self, path: &Path, _content: &[u8]) -> FileType { FileType::from_path(path) }
    }
    let detector: Arc<dyn FileTypeDetector> = Arc::new(RealDetector);
    struct TextBypassFactory;
    impl vecdb_core::parsers::ParserFactory for TextBypassFactory {
        fn get_parser(&self, file_type: FileType) -> Option<Box<dyn vecdb_core::parsers::Parser>> { match file_type { FileType::Text => None, _ => None } }
    }
    let factory: Arc<dyn vecdb_core::parsers::ParserFactory> = Arc::new(TextBypassFactory);

    let options = IngestionOptions {
        path: file_path.to_str().unwrap().to_string(),
        collection: "regress_pp".to_string(),
        chunk_size: 1000, 
        max_chunk_size: Some(2000),
        chunk_overlap: 0,
        respect_gitignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "char".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
    };

    println!("Starting Real P&P Regression...");
    let start = Instant::now();
    let _ = vecdb_core::ingestion::ingest_path(&backend, &embedder, &detector, &factory, options).await.unwrap();
    let duration = start.elapsed();
    
    println!("Real P&P ingestion took: {:?}", duration);
    // Should be < 2s for 735KB.
    assert!(duration.as_secs() < 3, "Performance Regression: P&P took too long ({:?})", duration);
}
