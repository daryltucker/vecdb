// PURPOSE:
//   Centralized error handling for vecq - the "jq for source code" tool.
//   Provides comprehensive error types covering parse errors, query errors, I/O errors,
//   and unsupported file types. Critical for vecq's reliability as users depend on
//   clear error messages when document parsing or querying fails.
//
// REQUIREMENTS:
//   User-specified:
//   - Must provide clear, actionable error messages for parsing failures
//   - Must distinguish between different error categories (parse, query, I/O)
//   - Must support error chaining to preserve root cause information
//   - Must integrate with jq-rs error types for query compatibility
//   
//   Implementation-discovered:
//   - Requires thiserror for ergonomic error handling and Display derivation
//   - Must support std::error::Error trait for error chaining
//   - Needs Send + Sync bounds for async compatibility
//   - Must preserve source location information for parse errors
//
// IMPLEMENTATION RULES:
//   1. Use thiserror for all error types to ensure consistent Display formatting
//      Rationale: Provides automatic Display implementation and error chaining
//   
//   2. Include file path and line number for all parse errors
//      Rationale: Users need to know exactly where parsing failed for debugging
//   
//   3. Provide suggestion field for query errors when possible
//      Rationale: Helps users fix invalid jq syntax with actionable guidance
//   
//   4. Use #[from] attribute for automatic error conversion from dependencies
//      Rationale: Reduces boilerplate and ensures proper error chaining
//   
//   Critical:
//   - DO NOT lose error context when converting between error types
//   - DO NOT expose internal implementation details in error messages
//   - ALWAYS include file path in parse errors for user debugging
//
// USAGE:
//   use vecq::error::{VecqError, VecqResult};
//   
//   // Parse error with location
//   return Err(VecqError::ParseError {
//       file: PathBuf::from("example.rs"),
//       line: 42,
//       message: "Unexpected token".to_string(),
//       source: Some(Box::new(syn_error)),
//   });
//   
//   // Query error with suggestion
//   return Err(VecqError::QueryError {
//       query: ".functions[".to_string(),
//       message: "Unclosed bracket".to_string(),
//       suggestion: Some("Try: .functions[]".to_string()),
//   });
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new error categories:
//   1. Add new variant to VecqError enum with descriptive fields
//   2. Add #[error("...")] attribute with user-friendly message format
//   3. Update error conversion From implementations if needed
//   4. Add unit tests in tests/unit/error_tests.rs
//   5. Update error handling documentation in README.md
//   
//   When parser dependencies change:
//   1. Update From implementations for new error types
//   2. Test error message formatting with new dependency versions
//   3. Ensure error chaining preserves all relevant context
//
// RELATED FILES:
//   - src/parser.rs - Uses ParseError for parser failures
//   - src/query.rs - Uses QueryError for jq syntax issues
//   - src/detection.rs - Uses UnsupportedFileType for unknown files
//   - src/main.rs - Handles error display and exit codes
//   - tests/unit/error_tests.rs - Error handling validation
//
// MAINTENANCE:
//   Update when:
//   - New parser dependencies are added (update From implementations)
//   - New error scenarios are discovered during testing
//   - User feedback indicates error messages need improvement
//   - jq-rs library updates change error types
//
// Last Verified: 2025-12-31

use std::path::PathBuf;
use thiserror::Error;

/// Comprehensive error type for all vecq operations
#[derive(Error, Debug)]
pub enum VecqError {
    /// Parse error with file location and context
    #[error("Parse error in {file} at line {line}: {message}")]
    ParseError {
        file: PathBuf,
        line: usize,
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Query error with jq syntax issues and suggestions
    #[error("Query error: {message}")]
    QueryError {
        query: String,
        message: String,
        suggestion: Option<String>,
    },

    /// I/O error for file operations
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    /// Unsupported file type error
    #[error("Unsupported file type: {file_type}")]
    UnsupportedFileType { file_type: String },

    /// JSON conversion error
    #[error("JSON conversion error: {message}")]
    JsonError {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Configuration error
    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    /// Cache error
    #[error("Cache error: {message}")]
    CacheError { message: String },

    /// Circuit breaker triggered due to complexity or size
    #[error("Circuit breaker triggered: {message}")]
    CircuitBreakerTriggered { message: String },
}

/// Result type alias for vecq operations
pub type VecqResult<T> = Result<T, VecqError>;

impl VecqError {
    /// Create a parse error with file context
    pub fn parse_error<E>(file: PathBuf, line: usize, message: String, source: Option<E>) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::ParseError {
            file,
            line,
            message,
            source: source.map(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
        }
    }

    /// Create a query error with optional suggestion
    pub fn query_error(query: String, message: String, suggestion: Option<String>) -> Self {
        Self::QueryError {
            query,
            message,
            suggestion,
        }
    }

    /// Create a JSON conversion error with source
    pub fn json_error<E>(message: String, source: Option<E>) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::JsonError {
            message,
            source: source.map(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
        }
    }

    /// Get user-friendly error message with suggestions
    pub fn user_message(&self) -> String {
        match self {
            VecqError::QueryError {
                message,
                suggestion: Some(suggestion),
                ..
            } => format!("{}\nSuggestion: {}", message, suggestion),
            _ => self.to_string(),
        }
    }

    /// Check if error is recoverable (can continue processing other files)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            VecqError::ParseError { .. }
                | VecqError::UnsupportedFileType { .. }
                | VecqError::JsonError { .. }
                | VecqError::CircuitBreakerTriggered { .. }
        )
    }
}

// Conversion from serde_json errors
impl From<serde_json::Error> for VecqError {
    fn from(err: serde_json::Error) -> Self {
        VecqError::json_error(
            format!("JSON serialization failed: {}", err),
            Some(err),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_error_creation() {
        let error = VecqError::parse_error(
            PathBuf::from("test.rs"),
            42,
            "Syntax error".to_string(),
            None::<std::io::Error>,
        );

        match error {
            VecqError::ParseError { file, line, message, .. } => {
                assert_eq!(file, PathBuf::from("test.rs"));
                assert_eq!(line, 42);
                assert_eq!(message, "Syntax error");
            }
            _ => panic!("Expected ParseError"),
        }
    }

    #[test]
    fn test_query_error_with_suggestion() {
        let error = VecqError::query_error(
            ".functions[".to_string(),
            "Unclosed bracket".to_string(),
            Some("Try: .functions[]".to_string()),
        );

        let user_message = error.user_message();
        assert!(user_message.contains("Unclosed bracket"));
        assert!(user_message.contains("Try: .functions[]"));
    }

    #[test]
    fn test_error_recoverability() {
        let parse_error = VecqError::parse_error(
            PathBuf::from("test.rs"),
            1,
            "Error".to_string(),
            None::<std::io::Error>,
        );
        assert!(parse_error.is_recoverable());

        let io_error = VecqError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "File not found",
        ));
        assert!(!io_error.is_recoverable());
    }
}