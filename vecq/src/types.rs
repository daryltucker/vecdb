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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use chrono::{DateTime, Utc};
use std::path::PathBuf;

/// Supported file types for parsing and conversion
pub use vecdb_common::FileType;

// Removed local FileType definition in favor of vecdb-common


// FileType implementation moved to vecdb-common


/// Types of structural elements found in documents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ElementType {
    // Universal elements
    Function,
    Class,
    Struct,
    Enum,
    Interface,
    Module,
    Import,
    Variable,
    Constant,
    Comment,

    // Markdown-specific
    Header,
    CodeBlock,
    Link,
    Table,
    List,
    Blockquote,
    Paragraph,
    HorizontalRule,
    Image,
    HtmlElement,

    // Language-specific
    Trait,        // Rust
    Implementation, // Rust impl blocks
    Decorator,    // Python
    Macro,        // Rust, C/C++
    Namespace,    // C++
    Package,      // Go
    Kernel,       // CUDA __global__
    DeviceFunction, // CUDA __device__

    // Generic container
    Block,
    Unknown,
}

impl fmt::Display for ElementType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Function => "function",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Interface => "interface",
            Self::Module => "module",
            Self::Import => "import",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Comment => "comment",
            Self::Header => "header",
            Self::CodeBlock => "code_block",
            Self::Link => "link",
            Self::Table => "table",
            Self::List => "list",
            Self::Blockquote => "blockquote",
            Self::Paragraph => "paragraph",
            Self::HorizontalRule => "horizontal_rule",
            Self::Image => "image",
            Self::HtmlElement => "element",
            Self::Trait => "trait",
            Self::Implementation => "implementation",
            Self::Decorator => "decorator",
            Self::Macro => "macro",
            Self::Namespace => "namespace",
            Self::Package => "package",
            Self::Kernel => "kernel",
            Self::DeviceFunction => "device_function",
            Self::Block => "block",
            Self::Unknown => "unknown",
        };
        write!(f, "{}", name)
    }
}

/// Metadata about a parsed document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub file_type: FileType,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub encoding: String,
    pub line_count: usize,
    pub hash: Option<String>, // For caching
}

impl DocumentMetadata {
    /// Create new document metadata
    pub fn new(path: PathBuf, size: u64) -> Self {
        let file_type = FileType::from_path(&path);
        Self {
            file_type,
            path,
            size,
            modified: None,
            encoding: "utf-8".to_string(),
            line_count: 0,
            hash: None,
        }
    }

    /// Update line count from content
    pub fn with_line_count(mut self, content: &str) -> Self {
        self.line_count = content.lines().count();
        self
    }

    /// Update modification time
    pub fn with_modified(mut self, modified: DateTime<Utc>) -> Self {
        self.modified = Some(modified);
        self
    }

    /// Update content hash for caching
    pub fn with_hash(mut self, hash: String) -> Self {
        self.hash = Some(hash);
        self
    }

    /// Override the file type (useful when path doesn't indicate type)
    pub fn with_file_type(mut self, file_type: FileType) -> Self {
        self.file_type = file_type;
        self
    }
}

/// Attributes specific to Rust elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustAttributes {
    pub visibility: String,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to TOML elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TomlAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to JavaScript elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JavaScriptAttributes {
    pub is_async: bool,
    pub is_arrow: bool,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to JSON elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Python elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonAttributes {
    pub is_async: bool,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Go elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to C/C++/CUDA elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CFamilyAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Bash elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Markdown elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to HTML elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to plain text elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Container for element-specific attributes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)] // Serializes content directly, no wrapping key
pub enum ElementAttributes {
    Rust(RustAttributes),
    Toml(TomlAttributes),
    JavaScript(JavaScriptAttributes),
    Json(JsonAttributes),
    Python(PythonAttributes),
    Go(GoAttributes),
    CFamily(CFamilyAttributes),
    Bash(BashAttributes),
    Markdown(MarkdownAttributes),
    Html(HtmlAttributes),
    Text(TextAttributes),
    Generic(HashMap<String, serde_json::Value>),
}

impl Default for ElementAttributes {
    fn default() -> Self {
        ElementAttributes::Generic(HashMap::new())
    }
}

impl ElementAttributes {
    /// Helper to insert into the generic map or flattened other map
    pub fn insert_generic(&mut self, key: String, value: serde_json::Value) {
        match self {
            Self::Generic(map) => { map.insert(key, value); },
            Self::Rust(attr) => { attr.other.insert(key, value); },
            Self::Toml(attr) => { attr.other.insert(key, value); },
            Self::JavaScript(attr) => { attr.other.insert(key, value); },
            Self::Json(attr) => { attr.other.insert(key, value); },
            Self::Python(attr) => { attr.other.insert(key, value); },
            Self::Go(attr) => { attr.other.insert(key, value); },
            Self::CFamily(attr) => { attr.other.insert(key, value); },
            Self::Bash(attr) => { attr.other.insert(key, value); },
            Self::Markdown(attr) => { attr.other.insert(key, value); },
            Self::Html(attr) => { attr.other.insert(key, value); },
            Self::Text(attr) => { attr.other.insert(key, value); },
        }
    }
    
    /// Helper to get generic value
    pub fn get_text(&self, key: &str) -> Option<String> {
        match self {
            Self::Generic(map) => map.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Rust(attr) => {
                if key == "visibility" { return Some(attr.visibility.clone()); }
                attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
            },
            Self::Toml(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::JavaScript(attr) => {
                if key == "is_async" { return Some(attr.is_async.to_string()); }
                if key == "is_arrow" { return Some(attr.is_arrow.to_string()); }
                attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
            },
            Self::Json(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Python(attr) => {
                if key == "is_async" { return Some(attr.is_async.to_string()); }
                attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
            },
            Self::Go(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::CFamily(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Bash(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Markdown(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Html(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Text(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
        }
    }

    /// Check if attributes are empty
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Generic(map) => map.is_empty(),
            Self::Rust(attr) => attr.visibility == "private" && attr.other.is_empty(), // Assume private is default?
            Self::Toml(attr) => attr.other.is_empty(),
            Self::JavaScript(attr) => !attr.is_async && !attr.is_arrow && attr.other.is_empty(),
            Self::Json(attr) => attr.other.is_empty(),
            Self::Python(attr) => !attr.is_async && attr.other.is_empty(),
            Self::Go(attr) => attr.other.is_empty(),
            Self::CFamily(attr) => attr.other.is_empty(),
            Self::Bash(attr) => attr.other.is_empty(),
            Self::Markdown(attr) => attr.other.is_empty(),
            Self::Html(attr) => attr.other.is_empty(),
            Self::Text(attr) => attr.other.is_empty(),
        }
    }

    /// Get generic value helper
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        match self {
            Self::Generic(map) => map.get(key),
            Self::Rust(attr) => attr.other.get(key),
            Self::Toml(attr) => attr.other.get(key),
            Self::JavaScript(attr) => attr.other.get(key),
            Self::Json(attr) => attr.other.get(key),
            Self::Python(attr) => attr.other.get(key),
            Self::Go(attr) => attr.other.get(key),
            Self::CFamily(attr) => attr.other.get(key),
            Self::Bash(attr) => attr.other.get(key),
            Self::Markdown(attr) => attr.other.get(key),
            Self::Html(attr) => attr.other.get(key),
            Self::Text(attr) => attr.other.get(key),
        }
    }

    /// Check if key exists
    pub fn contains_key(&self, key: &str) -> bool {
        match self {
            Self::Generic(map) => map.contains_key(key),
            Self::Rust(attr) => key == "visibility" || attr.other.contains_key(key),
            Self::Toml(attr) => attr.other.contains_key(key),
            Self::JavaScript(attr) => key == "is_async" || key == "is_arrow" || attr.other.contains_key(key),
            Self::Json(attr) => attr.other.contains_key(key),
            Self::Python(attr) => key == "is_async" || attr.other.contains_key(key),
            Self::Go(attr) => attr.other.contains_key(key),
            Self::CFamily(attr) => attr.other.contains_key(key),
            Self::Bash(attr) => attr.other.contains_key(key),
            Self::Markdown(attr) => attr.other.contains_key(key),
            Self::Html(attr) => attr.other.contains_key(key),
            Self::Text(attr) => attr.other.contains_key(key),
        }
    }
}

/// A structural element within a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentElement {
    pub element_type: ElementType,
    pub name: Option<String>,
    pub content: String,
    pub line_start: usize,
    pub line_end: usize,
    pub attributes: ElementAttributes,
    pub children: Vec<DocumentElement>,
}

impl DocumentElement {
    /// Create a new document element
    pub fn new(
        element_type: ElementType,
        name: Option<String>,
        content: String,
        line_start: usize,
        line_end: usize,
    ) -> Self {
        Self {
            element_type,
            name,
            content,
            line_start,
            line_end,
            attributes: ElementAttributes::default(),
            children: Vec::new(),
        }
    }

    /// Add an attribute to this element
    pub fn with_attribute<V: Into<serde_json::Value>>(mut self, key: String, value: V) -> Self {
        self.attributes.insert_generic(key, value.into());
        self
    }

    /// Add multiple attributes (legacy support - inserts into current variant)
    /// NOTE: This does NOT upgrade the variant. Use set_attributes for strict upgrading.
    pub fn with_attributes(mut self, attributes: HashMap<String, serde_json::Value>) -> Self {
        for (k, v) in attributes {
            self.attributes.insert_generic(k, v);
        }
        self
    }
    
    /// Set strict attributes
    pub fn set_attributes(mut self, attributes: ElementAttributes) -> Self {
        self.attributes = attributes;
        self
    }

    /// Add a child element
    pub fn with_child(mut self, child: DocumentElement) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple children
    pub fn with_children(mut self, children: Vec<DocumentElement>) -> Self {
        self.children.extend(children);
        self
    }

    /// Get the span of lines this element covers
    pub fn line_span(&self) -> std::ops::RangeInclusive<usize> {
        self.line_start..=self.line_end
    }

    /// Check if this element contains the given line number
    pub fn contains_line(&self, line: usize) -> bool {
        self.line_span().contains(&line)
    }

    /// Find all child elements of a specific type
    pub fn find_children(&self, element_type: ElementType) -> Vec<&DocumentElement> {
        let mut results = Vec::new();
        self.find_children_recursive(element_type, &mut results);
        results
    }

    fn find_children_recursive<'a>(&'a self, element_type: ElementType, results: &mut Vec<&'a DocumentElement>) {
        for child in &self.children {
            if child.element_type == element_type {
                results.push(child);
            }
            child.find_children_recursive(element_type, results);
        }
    }
}

/// Complete parsed document representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub metadata: DocumentMetadata,
    pub elements: Vec<DocumentElement>,
    #[serde(skip)]
    pub source_lines: Option<Vec<String>>,
}

impl ParsedDocument {
    /// Create a new parsed document
    pub fn new(metadata: DocumentMetadata) -> Self {
        Self {
            metadata,
            elements: Vec::new(),
            source_lines: None,
        }
    }

    /// Add an element to the document
    pub fn add_element(mut self, element: DocumentElement) -> Self {
        self.elements.push(element);
        self
    }

    /// Add multiple elements to the document
    pub fn add_elements(mut self, elements: Vec<DocumentElement>) -> Self {
        self.elements.extend(elements);
        self
    }

    /// Set source lines for context extraction
    pub fn with_source(mut self, content: &str) -> Self {
        self.source_lines = Some(content.lines().map(|s| s.to_string()).collect());
        self
    }

    /// Get context lines before a given line
    pub fn get_context_before(&self, line: usize, count: usize) -> Vec<String> {
        if let Some(lines) = &self.source_lines {
            // line is 1-indexed
            if line <= 1 {
                return Vec::new();
            }
            let end_idx = line - 1; // 0-indexed index of the line
            let start_idx = end_idx.saturating_sub(count);
            lines[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        }
    }

    /// Get context lines after a given line
    pub fn get_context_after(&self, line: usize, count: usize) -> Vec<String> {
        if let Some(lines) = &self.source_lines {
            // line is 1-indexed
            let start_idx = line; // 0-indexed index of the next line
            if start_idx >= lines.len() {
                return Vec::new();
            }
            let end_idx = std::cmp::min(start_idx + count, lines.len());
            lines[start_idx..end_idx].to_vec()
        } else {
            Vec::new()
        }
    }

    /// Find all elements of a specific type
    pub fn find_elements(&self, element_type: ElementType) -> Vec<&DocumentElement> {
        let mut results = Vec::new();
        for element in &self.elements {
            if element.element_type == element_type {
                results.push(element);
            }
            element.find_children_recursive(element_type, &mut results);
        }
        results
    }

    /// Get elements by line number
    pub fn elements_at_line(&self, line: usize) -> Vec<&DocumentElement> {
        self.elements
            .iter()
            .filter(|element| element.contains_line(line))
            .collect()
    }

    /// Get total line count
    pub fn line_count(&self) -> usize {
        self.metadata.line_count
    }

    /// Get file type
    pub fn file_type(&self) -> FileType {
        self.metadata.file_type
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_type_from_extension() {
        assert_eq!(FileType::from_extension("rs"), Some(FileType::Rust));
        assert_eq!(FileType::from_extension("py"), Some(FileType::Python));
        assert_eq!(FileType::from_extension("md"), Some(FileType::Markdown));
        assert_eq!(FileType::from_extension("cpp"), Some(FileType::Cpp));
        assert_eq!(FileType::from_extension("cu"), Some(FileType::Cuda));
        assert_eq!(FileType::from_extension("go"), Some(FileType::Go));
        assert_eq!(FileType::from_extension("sh"), Some(FileType::Bash));
        assert_eq!(FileType::from_extension("unknown"), None);
    }

    #[test]
    fn test_file_type_from_path() {
        assert_eq!(FileType::from_path("main.rs"), FileType::Rust);
        assert_eq!(FileType::from_path("script.py"), FileType::Python);
        assert_eq!(FileType::from_path("README.md"), FileType::Markdown);
        assert_eq!(FileType::from_path("unknown.xyz"), FileType::Unknown);
    }

    #[test]
    fn test_document_element_creation() {
        let element = DocumentElement::new(
            ElementType::Function,
            Some("main".to_string()),
            "fn main() {}".to_string(),
            1,
            1,
        )
        .with_attribute("visibility".to_string(), "public")
        .with_child(DocumentElement::new(
            ElementType::Variable,
            Some("x".to_string()),
            "let x = 42;".to_string(),
            2,
            2,
        ));

        assert_eq!(element.element_type, ElementType::Function);
        assert_eq!(element.name, Some("main".to_string()));
        assert_eq!(element.line_start, 1);
        assert_eq!(element.line_end, 1);
        assert_eq!(element.children.len(), 1);
        // With generic, this works via helper or check variant
        if let ElementAttributes::Generic(map) = element.attributes {
            assert!(map.contains_key("visibility"));
        } else {
            panic!("Expected Generic attributes");
        }
    }

    #[test]
    fn test_document_element_line_operations() {
        let element = DocumentElement::new(
            ElementType::Function,
            None,
            "content".to_string(),
            5,
            10,
        );

        assert_eq!(element.line_span(), 5..=10);
        assert!(element.contains_line(7));
        assert!(!element.contains_line(3));
        assert!(!element.contains_line(12));
    }

    #[test]
    fn test_parsed_document_operations() {
        let metadata = DocumentMetadata::new(PathBuf::from("test.rs"), 100)
            .with_line_count("line1\nline2\nline3");

        let doc = ParsedDocument::new(metadata)
            .add_element(DocumentElement::new(
                ElementType::Function,
                Some("func1".to_string()),
                "fn func1() {}".to_string(),
                1,
                1,
            ))
            .add_element(DocumentElement::new(
                ElementType::Function,
                Some("func2".to_string()),
                "fn func2() {}".to_string(),
                2,
                2,
            ));

        let functions = doc.find_elements(ElementType::Function);
        assert_eq!(functions.len(), 2);
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.file_type(), FileType::Rust);
    }
}