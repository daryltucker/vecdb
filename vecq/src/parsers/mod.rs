// PURPOSE:
//   Module organization for language-specific parsers in vecq.
//   Provides a centralized location for all parser implementations while
//   maintaining clean separation between different language parsers.
//   Essential for vecq's extensibility and maintainability.
//
// REQUIREMENTS:
//   User-specified:
//   - Must support all promised languages (Rust, Python, Markdown, C/C++, CUDA, Go, Bash)
//   - Must provide consistent interface across all parser implementations
//   - Must enable easy addition of new language parsers
//   - Must maintain parser isolation for independent development
//   
//   Implementation-discovered:
//   - Requires careful module organization for clean compilation
//   - Must handle conditional compilation for optional parsers
//   - Needs re-exports for convenient access to parser implementations
//   - Must support feature flags for reducing compilation time
//
// IMPLEMENTATION RULES:
//   1. Each language parser gets its own module file
//      Rationale: Enables independent development and testing of parsers
//   
//   2. All parsers must implement the Parser trait consistently
//      Rationale: Ensures uniform interface for parser registry
//   
//   3. Use feature flags for optional language support
//      Rationale: Reduces compilation time when only subset of languages needed
//   
//   4. Re-export parser implementations for convenient access
//      Rationale: Simplifies usage from other modules
//   
//   5. Provide parser factory functions for easy instantiation
//      Rationale: Hides implementation details from users
//   
//   Critical:
//   - DO NOT break Parser trait compatibility across implementations
//   - DO NOT create circular dependencies between parser modules
//   - ALWAYS maintain consistent error handling across all parsers
//
// USAGE:
//   use vecq::parsers::{MarkdownParser, RustParser, PythonParser};
//   
//   // Create parser instances
//   let markdown_parser = MarkdownParser::new();
//   let rust_parser = RustParser::new();
//   let python_parser = PythonParser::new();
//   
//   // Or use factory functions
//   let parser = vecq::parsers::create_parser(FileType::Rust)?;
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new language parser:
//   1. Create new module file (e.g., new_language.rs)
//   2. Implement Parser trait for new language
//   3. Add module declaration in this file
//   4. Add re-export for new parser struct
//   5. Update create_parser() factory function
//   6. Add comprehensive tests in tests/unit/parsers/
//   7. Update documentation and examples
//   
//   When modifying parser interface:
//   1. Update ALL parser implementations consistently
//   2. Update factory functions and re-exports
//   3. Update tests for all affected parsers
//   4. Document breaking changes in CHANGELOG.md
//   5. Provide migration guide for users
//
// RELATED FILES:
//   - src/parser.rs - Defines Parser trait that all parsers implement
//   - src/types.rs - Defines data structures used by parsers
//   - src/detection.rs - Uses parsers for file type processing
//   - tests/unit/parsers/ - Parser-specific unit tests
//   - tests/property/ - Property tests for parser correctness
//
// MAINTENANCE:
//   Update when:
//   - New language parsers are added
//   - Parser trait interface changes
//   - Feature flag configuration needs adjustment
//   - Parser dependencies need updates
//   - Performance optimization affects multiple parsers
//
// Last Verified: 2025-12-31

use crate::error::{VecqError, VecqResult};
use crate::parser::Parser;
use crate::types::FileType;

// Parser module declarations
pub mod markdown;
pub mod rust;

// Conditional compilation for optional parsers
pub mod python;

pub mod c;

pub mod cpp;

pub mod cuda;

pub mod go;

pub mod bash;

pub mod javascript;

pub mod text;
pub mod html;
pub mod toml;
pub mod json;

// Re-export parser implementations
pub use markdown::MarkdownParser;
pub use rust::RustParser;

pub use python::PythonParser;

pub use c::CParser;

pub use cpp::CppParser;

pub use cuda::CudaParser;

pub use go::GoParser;

pub use bash::BashParser;

pub use javascript::JavaScriptParser;

pub use text::TextParser;
pub use html::HtmlParser;
pub use toml::TomlParser;
pub use json::JsonParser;

/// Create a parser instance for the specified file type
pub fn create_parser(file_type: FileType) -> VecqResult<Box<dyn Parser>> {
    match file_type {

        FileType::Markdown => Ok(Box::new(MarkdownParser::new())),
        FileType::Rust => Ok(Box::new(RustParser::new())),
        
        FileType::Python => Ok(Box::new(PythonParser::new())),
        
        FileType::C => Ok(Box::new(CParser::new())),
        
        FileType::Cpp => Ok(Box::new(CppParser::new())),
        
        FileType::Cuda => Ok(Box::new(CudaParser::new())),
        
        FileType::Go => Ok(Box::new(GoParser::new())),
        
        FileType::Bash => Ok(Box::new(BashParser::new())),
        
        FileType::Text => Ok(Box::new(TextParser::new())),
        FileType::Html => Ok(Box::new(HtmlParser::new())),
        FileType::Toml => Ok(Box::new(TomlParser::new())),
        FileType::Json => Ok(Box::new(JsonParser::new())),
        
        _ => Err(VecqError::UnsupportedFileType {
            file_type: file_type.to_string(),
        }),
    }
}

/// Get list of available parsers (considering feature flags)
pub fn available_parsers() -> Vec<FileType> {
    let mut parsers = vec![
        FileType::Markdown,
        FileType::Rust,
    ];
    
    parsers.push(FileType::Python);
    
    parsers.push(FileType::C);
    
    parsers.push(FileType::Cpp);
    
    parsers.push(FileType::Cuda);
    
    parsers.push(FileType::Go);
    
    parsers.push(FileType::Bash);
    
    parsers.push(FileType::Text);
    parsers.push(FileType::Html);
    parsers.push(FileType::Toml);
    parsers.push(FileType::Json);
    
    parsers
}

/// Check if a parser is available for the given file type
pub fn is_parser_available(file_type: FileType) -> bool {
    available_parsers().contains(&file_type)
}

/// Get parser capabilities for a file type
pub fn get_parser_info(file_type: FileType) -> Option<ParserInfo> {
    match file_type {
        FileType::Markdown => Some(ParserInfo {
            name: "Markdown Parser".to_string(),
            version: "1.0".to_string(),
            supported_features: vec![
                "Headers".to_string(),
                "Code blocks".to_string(),
                "Links".to_string(),
                "Tables".to_string(),
                "Lists".to_string(),
            ],
            dependencies: vec!["pulldown-cmark".to_string()],
        }),
        FileType::Rust => Some(ParserInfo {
            name: "Rust Parser".to_string(),
            version: "1.0".to_string(),
            supported_features: vec![
                "Functions".to_string(),
                "Structs".to_string(),
                "Enums".to_string(),
                "Traits".to_string(),
                "Implementations".to_string(),
                "Comments".to_string(),
            ],
            dependencies: vec!["tree-sitter-rust".to_string()],
        }),
        FileType::Toml => Some(ParserInfo {
            name: "TOML Parser".to_string(),
            version: "1.0".to_string(),
            supported_features: vec![
                "Key-value pairs".to_string(),
                "Tables".to_string(),
                "Arrays".to_string(),
                "Inline tables".to_string(),
            ],
            dependencies: vec!["toml".to_string()],
        }),
        FileType::Json => Some(ParserInfo {
            name: "JSON Parser".to_string(),
            version: "1.0".to_string(),
            supported_features: vec![
                "Objects".to_string(),
                "Arrays".to_string(),
                "Strings".to_string(),
                "Numbers".to_string(),
                "Booleans".to_string(),
                "Null".to_string(),
            ],
            dependencies: vec!["serde_json".to_string()],
        }),
        _ => None,
    }
}

/// Information about a parser implementation
#[derive(Debug, Clone)]
pub struct ParserInfo {
    pub name: String,
    pub version: String,
    pub supported_features: Vec<String>,
    pub dependencies: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_parsers() {
        let parsers = available_parsers();
        assert!(parsers.contains(&FileType::Markdown));
        assert!(parsers.contains(&FileType::Rust));
        assert!(parsers.contains(&FileType::Json));
    }

    #[test]
    fn test_parser_availability() {
        assert!(is_parser_available(FileType::Markdown));
        assert!(is_parser_available(FileType::Rust));
        assert!(is_parser_available(FileType::Json));
        assert!(!is_parser_available(FileType::Unknown));
    }

    #[test]
    fn test_parser_creation() {
        let markdown_parser = create_parser(FileType::Markdown);
        assert!(markdown_parser.is_ok());
        
        let rust_parser = create_parser(FileType::Rust);
        assert!(rust_parser.is_ok());
        
        let unknown_parser = create_parser(FileType::Unknown);
        assert!(unknown_parser.is_err());
    }

    #[test]
    fn test_parser_info() {
        let markdown_info = get_parser_info(FileType::Markdown);
        assert!(markdown_info.is_some());
        let info = markdown_info.unwrap();
        assert_eq!(info.name, "Markdown Parser");
        assert!(!info.supported_features.is_empty());
        
        let rust_info = get_parser_info(FileType::Rust);
        assert!(rust_info.is_some());
        let info = rust_info.unwrap();
        assert_eq!(info.name, "Rust Parser");
        assert!(info.supported_features.contains(&"Functions".to_string()));
        
        let unknown_info = get_parser_info(FileType::Unknown);
        assert!(unknown_info.is_none());
    }
}