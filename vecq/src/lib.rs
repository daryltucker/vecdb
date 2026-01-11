// PURPOSE:
//   Main library interface for vecq - the "jq for source code" tool.
//   Provides clean public API for document parsing, JSON conversion, and querying.
//   Serves as the entry point for both the CLI binary and external library users.
//   Essential for maintaining API stability and backward compatibility.
//
// REQUIREMENTS:
//   User-specified:
//   - Must provide simple, intuitive API for parsing any structured document
//   - Must expose all core functionality (parsing, conversion, querying, formatting)
//   - Must maintain backward compatibility as the library evolves
//   - Must support both synchronous and asynchronous usage patterns
//   
//   Implementation-discovered:
//   - Requires careful module organization for clean API surface
//   - Must re-export key types and traits for user convenience
//   - Needs comprehensive documentation for all public interfaces
//   - Must handle feature flags for optional functionality
//
// IMPLEMENTATION RULES:
//   1. Re-export all essential types and traits at the crate root
//      Rationale: Provides convenient access without deep module paths
//   
//   2. Use feature flags for optional dependencies (property testing, etc.)
//      Rationale: Reduces compile time and binary size for basic usage
//   
//   3. Provide high-level convenience functions for common operations
//      Rationale: Makes the library easy to use for simple cases
//   
//   4. Maintain clear separation between public and private APIs
//      Rationale: Enables internal refactoring without breaking users
//   
//   5. Document all public APIs with examples and usage patterns
//      Rationale: Essential for library adoption and correct usage
//   
//   Critical:
//   - DO NOT expose internal implementation details in public API
//   - DO NOT break backward compatibility without major version bump
//   - ALWAYS provide comprehensive documentation for public functions
//
// USAGE:
//   // Basic usage - parse and query a file
//   use vecq::{parse_file, query_json, FileType};
//   
//   let content = std::fs::read_to_string("example.rs")?;
//   let parsed = parse_file(&content, FileType::Rust).await?;
//   let json = vecq::convert_to_json(parsed)?;
//   let result = query_json(&json, ".functions[] | select(.visibility == \"pub\")")?;
//   
//   // Advanced usage - custom parsers and formatters
//   use vecq::{ParserRegistry, FormatterRegistry, UnifiedJsonConverter};
//   
//   let mut parser_registry = ParserRegistry::new();
//   let converter = UnifiedJsonConverter::with_default_schemas();
//   let mut formatter_registry = FormatterRegistry::new();
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new modules:
//   1. Add module declaration with appropriate visibility
//   2. Re-export key types and traits at crate root if needed
//   3. Update high-level convenience functions to use new functionality
//   4. Add comprehensive documentation with examples
//   5. Update integration tests to cover new functionality
//   6. Consider feature flags for optional new dependencies
//   
//   When modifying public API:
//   1. Ensure backward compatibility or plan major version bump
//   2. Update all documentation and examples
//   3. Add deprecation warnings for removed functionality
//   4. Update CHANGELOG.md with breaking changes
//   5. Test with existing user code to validate compatibility
//
// RELATED FILES:
//   - src/main.rs - CLI binary that uses this library
//   - Cargo.toml - Defines library dependencies and features
//   - README.md - User-facing documentation and examples
//   - tests/ - Integration tests for public API
//   - examples/ - Usage examples for library users
//
// MAINTENANCE:
//   Update when:
//   - New core functionality is added to the library
//   - Public API needs modification or extension
//   - Feature flags need adjustment for optional functionality
//   - Documentation needs updates for new usage patterns
//   - Backward compatibility requirements change
//
// Last Verified: 2025-12-31

//! # vecq - jq for source code
//!
//! vecq is a tool that converts any structured document (source code, markdown, etc.)
//! into queryable JSON and enables jq-like querying with natural language support.
//!
//! ## Features
//!
//! - **Universal Document Parsing**: Support for Rust, Python, Markdown, C/C++, CUDA, Go, Bash
//! - **jq Compatibility**: 100% compatible with standard jq query syntax
//! - **Unix Pipeline Integration**: Grep-compatible output for seamless tool integration
//! - **Natural Language Queries**: Convert "List all functions" to jq syntax
//! - **Property-Based Testing**: Comprehensive correctness validation
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use vecq::{parse_file, convert_to_json, query_json, FileType};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Parse a Rust file
//!     let content = std::fs::read_to_string("src/lib.rs")?;
//!     let parsed = parse_file(&content, FileType::Rust).await?;
//!     
//!     // Convert to JSON
//!     let json = convert_to_json(parsed)?;
//!     
//!     // Query with jq syntax
//!     let functions = query_json(&json, ".functions[] | select(.visibility == \"pub\")")?;
//!     
//!     println!("{}", serde_json::to_string_pretty(&functions)?);
//!     Ok(())
//! }
//! ```
//!
//! ## Architecture
//!
//! vecq follows a modular architecture:
//!
//! 1. **File Type Detection** - Automatically identify document types
//! 2. **Language Parsers** - Extract structural information from documents
//! 3. **JSON Conversion** - Transform parsed structures to consistent JSON
//! 4. **Query Engine** - Process jq queries on JSON documents
//! 5. **Output Formatting** - Generate various output formats
//!
//! ## Advanced Usage
//!
//! For more control over the parsing and querying process:
//!
//! ```rust,no_run
//! use vecq::{ParserRegistry, UnifiedJsonConverter, JqQueryEngine, FormatterRegistry};
//! use vecq::{FileType, FormatOptions, JsonConverter, QueryEngine, OutputFormatter};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Set up components
//!     let parser_registry = ParserRegistry::new();
//!     let converter = UnifiedJsonConverter::with_default_schemas();
//!     let query_engine = JqQueryEngine::new();
//!     let formatter_registry = FormatterRegistry::new();
//!     
//!     // Get parser for file type
//!     let parser = parser_registry.get_parser(FileType::Rust)
//!         .ok_or("Rust parser not available")?;
//!     
//!     // Parse document
//!     let content = std::fs::read_to_string("example.rs")?;
//!     let parsed = parser.parse(&content).await?;
//!     
//!     // Convert to JSON
//!     let json = converter.convert(parsed)?;
//!     
//!     // Execute query
//!     let result = query_engine.execute_query(&json, ".functions[]")?;
//!     
//!     // Format output
//!     let formatter = formatter_registry.get_formatter("human")
//!         .ok_or("Human formatter not available")?;
//!     let options = FormatOptions::human_readable();
//!     let output = formatter.format(&result, &options)?;
//!     
//!     println!("{}", output);
//!     Ok(())
//! }
//! ```

// Core modules
pub mod error;
pub mod types;
pub mod parser;
pub mod converter;
pub mod generator;
pub mod generators;
pub mod query;
pub mod formatter;

// Parser implementations
pub mod parsers;

// File type detection
pub mod detection;

// Optional modules (behind feature flags)
#[cfg(feature = "natural-language")]
pub mod natural_language;
pub mod enrich;

// Re-export essential types and traits
pub use error::{VecqError, VecqResult};
pub use types::{ParsedDocument, DocumentElement, ElementType, FileType, DocumentMetadata};
pub use parser::{Parser, ParserRegistry, ParserCapabilities, ParserConfig};
pub use converter::{JsonConverter, UnifiedJsonConverter, SchemaRegistry};
pub use query::{QueryEngine, JqQueryEngine, QueryExplanation, QueryStats};
pub use formatter::{
    OutputFormatter, JsonFormatter, GrepFormatter, HumanFormatter,
    FormatterRegistry, FormatOptions
};
pub use detection::{FileTypeDetector, HybridDetector, DetectionConfig};
pub use enrich::Enricher;

// High-level convenience functions

/// Parse a file with automatic type detection
pub async fn parse_file_auto(content: &str, file_path: Option<&str>) -> VecqResult<ParsedDocument> {
    let file_type = if let Some(path) = file_path {
        FileType::from_path(path)
    } else {
        FileType::Unknown
    };
    
    parse_file(content, file_type).await
}

/// Parse a file with specified type
pub async fn parse_file(content: &str, file_type: FileType) -> VecqResult<ParsedDocument> {
    let registry = ParserRegistry::with_default_parsers()?;
    let parser = registry.get_parser(file_type)
        .ok_or_else(|| VecqError::UnsupportedFileType {
            file_type: file_type.to_string(),
        })?;
    
    let doc = parser.parse(content).await?;
    Ok(doc.with_source(content))
}

/// Enrich a document with post-parse content detection (D026)
pub fn enrich_document(doc: ParsedDocument) -> VecqResult<ParsedDocument> {
    let enricher = Enricher::new();
    enricher.enrich(doc)
}

/// Convert parsed document to JSON
pub fn convert_to_json(document: ParsedDocument) -> VecqResult<serde_json::Value> {
    let converter = UnifiedJsonConverter::with_default_schemas();
    converter.convert(document)
}

/// Execute jq query on JSON data
pub fn query_json(json: &serde_json::Value, query: &str) -> VecqResult<serde_json::Value> {
    let engine = JqQueryEngine::new();
    engine.execute_query(json, query)
}

/// Format query results for output
pub fn format_results(
    data: &serde_json::Value,
    format: &str,
    options: &FormatOptions,
) -> VecqResult<String> {
    let registry = FormatterRegistry::new();
    let formatter = registry.get_formatter(format)
        .ok_or_else(|| VecqError::ConfigError {
            message: format!("Unknown output format: {}", format),
        })?;
    
    formatter.format(data, options)
}

/// Complete workflow: parse file, convert to JSON, query, and format
pub async fn process_file(
    content: &str,
    file_type: FileType,
    query: &str,
    output_format: &str,
    options: &FormatOptions,
) -> VecqResult<String> {
    let json = if file_type == FileType::Toml {
         // Treat TOML as data for direct querying
         let toml_val: toml::Value = toml::from_str(content)
            .map_err(|e| VecqError::ParseError { 
                file: std::path::PathBuf::from("memory"),
                line: 0,
                message: e.to_string(),
                source: Some(Box::new(e))
            })?;
         // Convert to serde_json::Value
         // toml::Value implements serde::Serialize, so we can convert via generic serialization
         serde_json::to_value(toml_val).map_err(|e| VecqError::json_error("TOML to JSON conversion failed".to_string(), Some(e)))?
    } else if file_type == FileType::Json {
         // Treat JSON as data for direct querying
         serde_json::from_str(content).map_err(|e| VecqError::json_error("Invalid JSON input".to_string(), Some(e)))?
    } else {
        // Parse file
        let parsed = parse_file(content, file_type).await?;
        
        // Convert to JSON
        convert_to_json(parsed)?
    };
    
    // Execute query
    let result = query_json(&json, query)?;
    
    // Format output
    format_results(&result, output_format, options)
}

/// Validate jq query syntax
pub fn validate_query(query: &str) -> VecqResult<()> {
    // efficient: don't scan disk for scripts just to validate syntax
    let engine = JqQueryEngine::new_hermetic();
    engine.validate_query(query)
}

/// Get explanation of what a jq query does
pub fn explain_query(query: &str) -> VecqResult<QueryExplanation> {
    let engine = JqQueryEngine::new();
    engine.explain_query(query)
}

/// Get list of supported file types
pub fn supported_file_types() -> Vec<FileType> {
    vec![
        FileType::Markdown,
        FileType::Rust,
        FileType::Python,
        FileType::Html,
        FileType::C,
        FileType::Cpp,
        FileType::Cuda,
        FileType::Go,
        FileType::Bash,
        FileType::Json,
    ]
}

/// Get list of available output formats
pub fn available_output_formats() -> Vec<String> {
    let registry = FormatterRegistry::new();
    registry.available_formatters()
}

/// Check if a file type is supported
pub fn is_file_type_supported(file_type: FileType) -> bool {
    file_type.is_supported()
}

/// Get file type from file extension
pub fn detect_file_type(file_path: &str) -> FileType {
    FileType::from_path(file_path)
}

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NAME: &str = env!("CARGO_PKG_NAME");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_file_type_detection() {
        assert_eq!(detect_file_type("main.rs"), FileType::Rust);
        assert_eq!(detect_file_type("script.py"), FileType::Python);
        assert_eq!(detect_file_type("README.md"), FileType::Markdown);
        assert_eq!(detect_file_type("unknown.xyz"), FileType::Unknown);
    }

    #[test]
    fn test_supported_file_types() {
        let types = supported_file_types();
        assert!(types.contains(&FileType::Rust));
        assert!(types.contains(&FileType::Python));
        assert!(types.contains(&FileType::Markdown));
        assert!(!types.contains(&FileType::Unknown));
    }

    #[test]
    fn test_file_type_support_check() {
        assert!(is_file_type_supported(FileType::Rust));
        assert!(is_file_type_supported(FileType::Python));
        assert!(!is_file_type_supported(FileType::Unknown));
    }

    #[test]
    fn test_available_output_formats() {
        let formats = available_output_formats();
        assert!(formats.contains(&"json".to_string()));
        assert!(formats.contains(&"grep".to_string()));
        assert!(formats.contains(&"human".to_string()));
    }

    #[tokio::test]
    async fn test_query_validation() {
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            assert!(validate_query(".").is_ok());
            assert!(validate_query(".functions").is_ok());
            assert!(validate_query("").is_err());
            assert!(validate_query(".functions[").is_err());
        }).await;
        
        assert!(result.is_ok(), "Test timed out! Infinite recursion likely.");
    }

    #[test]
    fn test_query_explanation() {
        let explanation = explain_query(".functions[] | select(.visibility == \"pub\")").unwrap();
        assert_eq!(explanation.query, ".functions[] | select(.visibility == \"pub\")");
        assert!(!explanation.operations.is_empty());
    }

    #[test]
    fn test_json_querying() {
        let data = json!({
            "functions": [
                {"name": "main", "visibility": "pub"},
                {"name": "helper", "visibility": "private"}
            ]
        });

        let result = query_json(&data, ".functions").unwrap();
        assert!(result.is_array());
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_result_formatting() {
        let data = json!([
            {"name": "test", "line_start": 1, "content": "test content"}
        ]);

        let options = FormatOptions::default();
        
        // JSON format
        let json_output = format_results(&data, "json", &options).unwrap();
        assert!(json_output.contains("test"));
        
        // Grep format
        let grep_output = format_results(&data, "grep", &options).unwrap();
        assert!(grep_output.contains(":1:"));
    }

    #[test]
    fn test_version_constants() {
        assert!(!VERSION.is_empty());
        assert_eq!(NAME, "vecq");
        assert!(!DESCRIPTION.is_empty());
    }
}