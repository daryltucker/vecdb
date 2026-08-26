use crate::types::Chunk;
use anyhow::Result;
use std::path::Path;

use async_trait::async_trait;

/// Trait for content-aware parsers
#[async_trait]
pub trait Parser: Send + Sync {
    /// Parse the file content and return chunks
    async fn parse(
        &self,
        content: &str,
        path: &Path,
        base_metadata: Option<serde_json::Value>,
    ) -> Result<Vec<Chunk>>;

    /// Get the file extensions supported by this parser
    fn supported_extensions(&self) -> Vec<&str>;
}

// vecdb-core no longer carries its own per-file-type parsers.
//
// It used to ship `json.rs` (a JSON/JSON5 parser) and `yaml.rs` (which served
// FileType::Toml), reachable only through `BuiltinParserFactory` — which no
// binary ever constructed. vecq had its own JSON and TOML parsers, and those
// were the ones that actually ran. Two implementations, different support
// levels, and the core test suite pointed at the unreachable pair: it asserted
// that JSON-with-comments worked while every real `tsconfig.json` silently
// degraded to one unstructured text chunk.
//
// Language and format support lives in vecq, once. The JSON5 capability moved
// there with it (`vecq/tests/tier1_json_comments.rs`).
//
// `streaming_json` stays: it is not a second opinion about JSON structure, it is
// the only way to read a file too large to hold in memory as an AST. It is
// reachable, via `get_streaming_parser`.
pub mod streaming_json;
// The one vecq->chunk bridge. It was once "moved to the CLI layer" to satisfy a
// rule that vecdb-core must not depend on vecq, and that move is how it came to
// exist twice: the server grew its own copy and the two drifted apart — chunk IDs
// keyed on line numbers, a missing redundancy filter, no streaming JSON parser.
// The rule is retired; the adapter is shared. See tier1_single_vecq_adapter.rs.
pub mod vecq_adapter;

use vecdb_common::FileType;

/// Factory for creating parsers (dependency injection interface)
pub trait ParserFactory: Send + Sync {
    /// Get a parser for a specific file type
    fn get_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>>;

    /// Get a streaming parser for a specific file type (for large files)
    fn get_streaming_parser(&self, _file_type: FileType) -> Option<Box<dyn Parser>> {
        None
    }
}

// `BuiltinParserFactory` was here.
//
// It mapped FileType::Json to vecdb-core's own JSON parser and FileType::Toml to
// a *YamlParser*, and nothing in any binary ever constructed it — all nineteen
// injection sites use `VecqParserFactory`. It existed to be tested: the core
// parser-compliance suite exercised it and nothing else, which is why that suite
// never noticed that vecq's Python and Go parsers emitted signature stubs
// instead of code.
//
// A parser that no binary can reach is not a fallback, it is a second opinion
// nobody asked for. There is now one implementation per file type, in vecq.
