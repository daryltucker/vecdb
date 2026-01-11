// PURPOSE:
//   Generic text parser implementation for vecq.
//   Handles files that don't have a specific language parser (txt, log, json, yaml, etc.).
//   Treats content as raw text blocks, splitting by lines but not building an AST.
//
// RELATED FILES:
//   - src/parsers/mod.rs - Parser registry
//   - src/types.rs - DocumentElement, ElementType definitions

use crate::error::VecqResult;
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument};
use async_trait::async_trait;
use std::path::PathBuf;

/// Generic text parser that treats content as raw text
#[derive(Debug, Clone)]
pub struct TextParser {
    _config: ParserConfig,
}

impl TextParser {
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }
}

impl Default for TextParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for TextParser {
    fn file_extensions(&self) -> &[&str] {
        &["txt", "log", "cfg", "ini", "conf", "yaml", "yml", "json", "toml", "xml"]
    }

    fn language_name(&self) -> &str {
        "Text"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: false,
            type_information: false,
            macros: false,
            max_file_size: None, // No limit by default
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // For generic text, we create one large 'Block' element containing the content.
        // In the future, we could split by paragraphs or double newlines.
        
        let line_count = content.lines().count();
        
        let element = DocumentElement::new(
            ElementType::Block,
            None, // No name for a generic block
            content.to_string(),
            1,
            line_count.max(1),
        );

        let mut elements = Vec::new();
        if !content.is_empty() {
            elements.push(element);
        }

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.txt"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Text)
        );
        doc.elements = elements;

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_simple_text() {
        let parser = TextParser::new();
        let content = "Line 1\nLine 2\nLine 3";
        let result = parser.parse(content).await.unwrap();
        
        assert_eq!(result.elements.len(), 1);
        assert_eq!(result.metadata.file_type, FileType::Text);
        assert_eq!(result.elements[0].content, content);
        assert_eq!(result.elements[0].line_start, 1);
        assert_eq!(result.elements[0].line_end, 3);
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = TextParser::new();
        let content = "";
        let result = parser.parse(content).await.unwrap();
        
        assert!(result.elements.is_empty());
        assert_eq!(result.metadata.file_type, FileType::Text);
    }

    #[tokio::test]
    async fn test_single_line() {
        let parser = TextParser::new();
        let content = "Single line";
        let result = parser.parse(content).await.unwrap();
        
        assert_eq!(result.elements.len(), 1);
        assert_eq!(result.elements[0].line_start, 1);
        assert_eq!(result.elements[0].line_end, 1);
    }
}
