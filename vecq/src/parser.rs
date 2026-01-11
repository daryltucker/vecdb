// PURPOSE:
//   Core Parser trait that all language parsers must implement for vecq.
//   Provides the common interface that enables vecq's universal document processing
//   while allowing each language parser to handle its specific syntax and semantics.
//   Essential for vecq's extensibility - new language support requires only
//   implementing this trait.
//
// REQUIREMENTS:
//   User-specified:
//   - Must provide consistent interface across all supported languages
//   - Must support async parsing for large files
//   - Must handle malformed input gracefully without crashing
//   - Must preserve all structural information and line numbers
//   
//   Implementation-discovered:
//   - Requires Send + Sync for multi-threaded processing
//   - Must return Result type for error handling
//   - Needs Clone for parser registry management
//   - Must support both streaming and batch parsing modes
//
// IMPLEMENTATION RULES:
//   1. All parsers must implement the Parser trait with consistent error handling
//      Rationale: Enables uniform error reporting and recovery across all file types
//   
//   2. parse() method must never panic, always return Result
//      Rationale: Malformed files should not crash vecq, must degrade gracefully
//   
//   3. file_extensions() must return all supported extensions for the language
//      Rationale: Used by file type detection for accurate parser selection
//   
//   4. Parsers must preserve exact line numbers for all structural elements
//      Rationale: Required for grep compatibility and source location tracking
//   
//   5. Use ParsedDocument as the universal output format
//      Rationale: Enables consistent JSON conversion regardless of source language
//   
//   Critical:
//   - DO NOT change Parser trait interface without migration plan for all parsers
//   - DO NOT allow parsers to panic on malformed input
//   - ALWAYS preserve line number accuracy in parsed elements
//
// USAGE:
//   use vecq::parser::{Parser, ParserRegistry};
//   use vecq::types::{FileType, ParsedDocument};
//   
//   // Implement parser for new language
//   struct MyLanguageParser;
//   
//   impl Parser for MyLanguageParser {
//       fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
//           // Parse content and return structured document
//       }
//       
//       fn file_extensions(&self) -> &[&str] {
//           &["mylang", "ml"]
//       }
//       
//       fn language_name(&self) -> &str {
//           "MyLanguage"
//       }
//   }
//   
//   // Register and use parser
//   let mut registry = ParserRegistry::new();
//   registry.register(FileType::Unknown, Box::new(MyLanguageParser));
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new parser implementations:
//   1. Implement all three trait methods (parse, file_extensions, language_name)
//   2. Add comprehensive error handling for malformed input
//   3. Ensure line number preservation in all DocumentElements
//   4. Add parser to ParserRegistry in src/detection.rs
//   5. Add unit tests in tests/unit/parsers/
//   6. Add property tests for new language in tests/property/
//   
//   When modifying Parser trait:
//   1. Update ALL existing parser implementations
//   2. Update ParserRegistry to handle new trait methods
//   3. Update property tests to validate new trait requirements
//   4. Document breaking changes in CHANGELOG.md
//   5. Provide migration guide for external parser implementations
//
// RELATED FILES:
//   - src/types.rs - Defines ParsedDocument and DocumentElement structures
//   - src/error.rs - Defines VecqError and VecqResult types
//   - src/detection.rs - Uses Parser trait for file type processing
//   - src/parsers/*.rs - All language-specific parser implementations
//   - tests/unit/parser_tests.rs - Parser trait validation tests
//
// MAINTENANCE:
//   Update when:
//   - New language parsers are added to the system
//   - Parser interface needs extension for new features
//   - Error handling requirements change
//   - Performance optimization needs new trait methods
//   - Async parsing support needs enhancement
//
// Last Verified: 2025-12-31

use crate::error::VecqResult;
use crate::types::{FileType, ParsedDocument};
use async_trait::async_trait;
use std::collections::HashMap;

/// Core trait that all language parsers must implement
#[async_trait]
pub trait Parser: Send + Sync {
    /// Parse content and return structured document representation
    /// 
    /// This method must handle malformed input gracefully and never panic.
    /// It should extract as much structural information as possible even
    /// from partially valid files.
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument>;

    /// Get file extensions supported by this parser
    /// 
    /// Used by file type detection to select the appropriate parser.
    /// Should include all common extensions for the language.
    fn file_extensions(&self) -> &[&str];

    /// Get human-readable language name
    /// 
    /// Used for error messages and user-facing output.
    fn language_name(&self) -> &str;

    /// Get parser capabilities and features
    /// 
    /// Optional method to describe what language features this parser supports.
    /// Used for documentation and feature detection.
    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities::default()
    }

    /// Validate content before parsing (optional optimization)
    /// 
    /// Quick validation to check if content is likely parseable.
    /// Should be fast and conservative (false positives OK, false negatives not).
    fn can_parse(&self, content: &str) -> bool {
        // Default implementation: assume all content is parseable
        !content.trim().is_empty()
    }

    /// Get parser configuration options
    /// 
    /// Returns configuration that affects parsing behavior.
    /// Used for caching and parser selection optimization.
    fn config(&self) -> ParserConfig {
        ParserConfig::default()
    }
}

/// Parser capabilities and supported features
#[derive(Debug, Clone, Default)]
pub struct ParserCapabilities {
    /// Supports incremental parsing for large files
    pub incremental: bool,
    /// Supports parsing with syntax errors (error recovery)
    pub error_recovery: bool,
    /// Supports extracting documentation comments
    pub documentation: bool,
    /// Supports extracting type information
    pub type_information: bool,
    /// Supports extracting macro/preprocessor information
    pub macros: bool,
    /// Maximum file size this parser can handle efficiently (bytes)
    pub max_file_size: Option<usize>,
}

/// Parser configuration options
#[derive(Debug, Clone)]
pub struct ParserConfig {
    /// Whether to preserve whitespace in parsed content
    pub preserve_whitespace: bool,
    /// Whether to extract comments as separate elements
    pub extract_comments: bool,
    /// Whether to resolve imports/includes
    pub resolve_imports: bool,
    /// Custom parser options
    pub custom_options: HashMap<String, serde_json::Value>,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self {
            preserve_whitespace: false,
            extract_comments: true,
            resolve_imports: false,
            custom_options: HashMap::new(),
        }
    }
}

/// Registry for managing parser instances
#[derive(Default)]
pub struct ParserRegistry {
    parsers: HashMap<FileType, Box<dyn Parser>>,
}

impl ParserRegistry {
    /// Create a new parser registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new parser registry with all available parsers
    pub fn with_default_parsers() -> VecqResult<Self> {
        let mut registry = Self::new();
        
        // Register all available parsers
        for file_type in crate::parsers::available_parsers() {
            let parser = crate::parsers::create_parser(file_type)?;
            registry.register(file_type, parser);
        }
        
        Ok(registry)
    }

    /// Register a parser for a specific file type
    pub fn register(&mut self, file_type: FileType, parser: Box<dyn Parser>) {
        self.parsers.insert(file_type, parser);
    }

    /// Get parser for a specific file type
    pub fn get_parser(&self, file_type: FileType) -> Option<&dyn Parser> {
        self.parsers.get(&file_type).map(|p| p.as_ref())
    }

    /// Check if a file type is supported
    pub fn supports(&self, file_type: FileType) -> bool {
        self.parsers.contains_key(&file_type)
    }

    /// Get all supported file types
    pub fn supported_types(&self) -> Vec<FileType> {
        self.parsers.keys().copied().collect()
    }

    /// Get parser capabilities for a file type
    pub fn get_capabilities(&self, file_type: FileType) -> Option<ParserCapabilities> {
        self.get_parser(file_type).map(|p| p.capabilities())
    }
}

/// Utility functions for parser implementations
pub mod utils {
    use crate::types::{DocumentElement, ElementType};

    /// Calculate line number from byte offset in content
    pub fn line_number_from_offset(content: &str, offset: usize) -> usize {
        content[..offset.min(content.len())]
            .chars()
            .filter(|&c| c == '\n')
            .count() + 1
    }

    /// Fast line counter using pre-calculated offsets
    pub use vecdb_common::LineCounter;

    /// Extract line range for a span of text
    pub fn line_range_from_span(content: &str, start: usize, end: usize) -> (usize, usize) {
        let start_line = line_number_from_offset(content, start);
        let end_line = line_number_from_offset(content, end);
        (start_line, end_line)
    }

    /// Create a document element with automatic line number calculation
    pub fn create_element_with_span(
        content: &str,
        element_type: ElementType,
        name: Option<String>,
        element_content: String,
        start_offset: usize,
        end_offset: usize,
    ) -> DocumentElement {
        let (line_start, line_end) = line_range_from_span(content, start_offset, end_offset);
        DocumentElement::new(element_type, name, element_content, line_start, line_end)
    }

    /// Sanitize content for safe processing
    pub fn sanitize_content(content: &str) -> String {
        // Remove null bytes and other problematic characters
        content
            .chars()
            .filter(|&c| c != '\0' && c.is_control() == (c == '\n' || c == '\r' || c == '\t'))
            .collect()
    }

    /// Check if content appears to be binary
    pub fn is_likely_binary(content: &[u8]) -> bool {
        // Simple heuristic: if more than 30% of bytes are non-printable, likely binary
        let non_printable = content
            .iter()
            .take(1024) // Check first 1KB
            .filter(|&&b| b < 32 && b != b'\n' && b != b'\r' && b != b'\t')
            .count();
        
        non_printable as f64 / content.len().min(1024) as f64 > 0.3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocumentElement, DocumentMetadata, ElementType};
    use std::path::PathBuf;

    // Mock parser for testing
    struct MockParser {
        language: String,
        extensions: Vec<&'static str>,
    }

    #[async_trait]
    impl Parser for MockParser {
        async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
            let metadata = DocumentMetadata::new(PathBuf::from("test.mock"), content.len() as u64)
                .with_line_count(content);

            let element = DocumentElement::new(
                ElementType::Function,
                Some("mock_function".to_string()),
                content.to_string(),
                1,
                content.lines().count(),
            );

            Ok(ParsedDocument::new(metadata).add_element(element))
        }

        fn file_extensions(&self) -> &[&str] {
            &self.extensions
        }

        fn language_name(&self) -> &str {
            &self.language
        }
    }

    #[tokio::test]
    async fn test_parser_registry() {
        let mut registry = ParserRegistry::new();
        
        let mock_parser = MockParser {
            language: "Mock".to_string(),
            extensions: vec!["mock"],
        };

        registry.register(FileType::Unknown, Box::new(mock_parser));

        assert!(registry.supports(FileType::Unknown));
        assert!(!registry.supports(FileType::Rust));

        let parser = registry.get_parser(FileType::Unknown).unwrap();
        assert_eq!(parser.language_name(), "Mock");
        assert_eq!(parser.file_extensions(), &["mock"]);
    }

    #[tokio::test]
    async fn test_mock_parser() {
        let parser = MockParser {
            language: "Mock".to_string(),
            extensions: vec!["mock"],
        };

        let content = "line 1\nline 2\nline 3";
        let result = parser.parse(content).await.unwrap();

        assert_eq!(result.elements.len(), 1);
        assert_eq!(result.elements[0].element_type, ElementType::Function);
        assert_eq!(result.elements[0].name, Some("mock_function".to_string()));
        assert_eq!(result.line_count(), 3);
    }

    #[test]
    fn test_parser_utils() {
        let content = "line 1\nline 2\nline 3\n";
        
        assert_eq!(utils::line_number_from_offset(content, 0), 1);
        assert_eq!(utils::line_number_from_offset(content, 7), 2);
        assert_eq!(utils::line_number_from_offset(content, 14), 3);

        let (start, end) = utils::line_range_from_span(content, 0, 6);
        assert_eq!(start, 1);
        assert_eq!(end, 1);

        let (start, end) = utils::line_range_from_span(content, 0, 14);
        assert_eq!(start, 1);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_content_sanitization() {
        let dirty_content = "hello\0world\x01test";
        let clean_content = utils::sanitize_content(dirty_content);
        assert_eq!(clean_content, "helloworldtest");

        let normal_content = "hello\nworld\ttest";
        let unchanged = utils::sanitize_content(normal_content);
        assert_eq!(unchanged, normal_content);
    }

    #[test]
    fn test_binary_detection() {
        let text_content = b"This is normal text content\nwith newlines";
        assert!(!utils::is_likely_binary(text_content));

        let binary_content = vec![0u8; 100]; // All null bytes
        assert!(utils::is_likely_binary(&binary_content));

        let mixed_content = b"Some text\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09";
        assert!(utils::is_likely_binary(mixed_content));
    }
}