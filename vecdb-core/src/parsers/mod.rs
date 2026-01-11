use crate::types::Chunk;
use anyhow::Result;
use std::path::Path;

use async_trait::async_trait;

/// Trait for content-aware parsers
#[async_trait]
pub trait Parser: Send + Sync {
    /// Parse the file content and return chunks
    async fn parse(&self, content: &str, path: &Path, base_metadata: Option<serde_json::Value>) -> Result<Vec<Chunk>>;
    
    /// Get the file extensions supported by this parser
    fn supported_extensions(&self) -> Vec<&str>;
}


pub mod json;

pub mod yaml;
pub mod streaming_json;
// pub mod vecq_adapter; // Moved to CLI layer


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

/// Default built-in factory for generic types (JSON, YAML)
pub struct BuiltinParserFactory;

impl ParserFactory for BuiltinParserFactory {
    fn get_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        match file_type {
            FileType::Json => Some(Box::new(json::JsonParser::new())),
            FileType::Toml => Some(Box::new(yaml::YamlParser::new())),
            // Code types are handled by external adapters (vecq)
            _ => None,
        }
    }

    fn get_streaming_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        match file_type {
            FileType::Json => Some(Box::new(streaming_json::StreamingJsonParser::new())),
            _ => None,
        }
    }
}

