// PURPOSE:
//   Core type definitions for vecq file type detection and document representation.
//   Defines the FileType enum that drives parser selection and the fundamental
//   data structures for representing parsed documents. Essential for vecq's
//   architecture as it enables the universal JSON conversion while preserving
//   type-specific structural information.
//
// REQUIREMENTS:
//   User-specified:
//   - Must support Markdown, Rust, Python, C, C++, CUDA, Go, and Bash file types
//   - Must provide consistent document representation across all file types
//   - Must preserve line number information for grep compatibility
//   - Must support hierarchical document structures (headers, nested elements)
//   
//   Implementation-discovered:
//   - Requires Serialize/Deserialize for JSON conversion
//   - Must support Clone for caching and multiple processing passes
//   - Needs Debug for development and error reporting
//   - Must be Send + Sync for async processing
//
// IMPLEMENTATION RULES:
//   1. FileType enum must be exhaustive for all supported languages
//      Rationale: Parser selection depends on complete type coverage
//   
//   2. DocumentElement must support arbitrary nesting via children field
//      Rationale: Enables representation of complex hierarchical structures
//   
//   3. All structural elements must include line_start and line_end
//      Rationale: Required for grep compatibility and source location tracking
//   
//   4. Use HashMap for attributes to support language-specific metadata
//      Rationale: Different languages have different attribute types (Rust derives, Python decorators)
//   
//   5. ElementType must be extensible for new language constructs
//      Rationale: New parsers may discover additional structural elements
//   
//   Critical:
//   - DO NOT remove existing FileType variants (breaks backward compatibility)
//   - DO NOT change DocumentElement structure without migration plan
//   - ALWAYS preserve line number information in all elements
//
// USAGE:
//   use vecq::types::{FileType, ParsedDocument, DocumentElement, ElementType};
//   
//   // File type detection
//   let file_type = FileType::from_extension("rs");
//   assert_eq!(file_type, Some(FileType::Rust));
//   
//   // Document creation
//   let doc = ParsedDocument {
//       file_type: FileType::Rust,
//       metadata: DocumentMetadata::new(path, content.len()),
//       elements: vec![
//           DocumentElement {
//               element_type: ElementType::Function,
//               name: Some("main".to_string()),
//               content: "fn main() {}".to_string(),
//               line_start: 1,
//               line_end: 1,
//               attributes: ElementAttributes::default(),
//               children: vec![],
//           }
//       ],
//   };
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new file types:
//   1. Add new variant to FileType enum
//   2. Update from_extension() method with new file extensions
//   3. Add display name to Display implementation
//   4. Update file_extensions() method
//   5. Add parser implementation in src/parsers/
//   6. Update tests in tests/unit/types_tests.rs
//   
//   When adding new element types:
//   1. Add variant to ElementType enum
//   2. Update Display implementation
//   3. Add parser support for new element type
//   4. Update JSON schema documentation
//   5. Add property tests for new element type
//
// RELATED FILES:
//   - src/detection.rs - Uses FileType for parser selection
//   - src/parser.rs - Creates ParsedDocument structures
//   - src/converter.rs - Converts ParsedDocument to JSON
//   - src/parsers/*.rs - Language-specific parser implementations
//   - tests/unit/types_tests.rs - Type definition validation
//
// MAINTENANCE:
//   Update when:
//   - New programming languages need support
//   - New structural elements are discovered in existing languages
//   - Document metadata requirements change
//   - JSON schema needs evolution for new features
//
// Last Verified: 2025-12-31

pub mod element_type;
pub mod attributes;
pub mod metadata;
pub mod document;

pub use vecdb_common::FileType;
pub use element_type::ElementType;
pub use attributes::*;
pub use metadata::DocumentMetadata;
pub use document::{DocumentElement, ParsedDocument};