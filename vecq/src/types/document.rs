use super::attributes::ElementAttributes;
use super::element_type::ElementType;
use super::metadata::DocumentMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use vecdb_common::FileType;
// removed

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

    fn find_children_recursive<'a>(
        &'a self,
        element_type: ElementType,
        results: &mut Vec<&'a DocumentElement>,
    ) {
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
    use std::path::PathBuf;

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
        let element =
            DocumentElement::new(ElementType::Function, None, "content".to_string(), 5, 10);

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
