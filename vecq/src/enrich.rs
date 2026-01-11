use crate::types::{ParsedDocument, DocumentElement, FileType, ElementType, ElementAttributes};
use crate::error::VecqResult;
use crate::parsers::markdown::parse_markdown_document;

/// Result of a content detection attempt
#[derive(Debug, Clone, PartialEq)]
pub enum DetectionResult {
    /// No specific content type detected
    None,
    /// Detected a specific file type
    Detected(FileType),
}

/// Utility for performing post-parse content detection and enrichment.
/// (D026: Lazy Content Resolution)
pub struct Enricher {
    max_depth: usize,
    heuristic_window: usize,
}

impl Default for Enricher {
    fn default() -> Self {
        Self {
            max_depth: 10,
            heuristic_window: 2048,
        }
    }
}

impl Enricher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enrich a document by recursively detecting and parsing sub-languages.
    pub fn enrich(&self, mut doc: ParsedDocument) -> VecqResult<ParsedDocument> {
        let mut enriched_elements = Vec::new();
        for el in doc.elements {
            enriched_elements.push(self.enrich_element(el, 0)?);
        }
        doc.elements = enriched_elements;
        Ok(doc)
    }

    fn enrich_element(&self, mut el: DocumentElement, depth: usize) -> VecqResult<DocumentElement> {
        if depth >= self.max_depth {
            return Ok(el);
        }

        // 1. Detect content type
        let detected = self.detect_content_type(&el);

        // 2. If it's Markdown, parse it and add as children
        if let DetectionResult::Detected(FileType::Markdown) = detected {
            // Check if it already has markdown children to prevent double-enrichment
            let already_enriched = el.children.iter().any(|c| 
                matches!(c.element_type, ElementType::Header | ElementType::List | ElementType::CodeBlock)
            );

            if !already_enriched {
                let md_doc = parse_markdown_document(&el.content);
                let mut md_elements = md_doc.elements;
                
                // Offset line numbers based on parent's start line
                let offset = el.line_start.saturating_sub(1);
                self.offset_elements(&mut md_elements, offset);
                
                el.children.extend(md_elements);

                // Set metadata reflecting the enrichment
                if let ElementAttributes::Html(ref mut html) = el.attributes {
                    html.other.insert(
                        "x-detected-content-type".to_string(),
                        serde_json::Value::String("Markdown".to_string())
                    );
                }
            }
        }

        // 3. Recurse into children
        let mut enriched_children = Vec::new();
        for child in el.children {
            enriched_children.push(self.enrich_element(child, depth + 1)?);
        }
        el.children = enriched_children;

        Ok(el)
    }

    fn offset_elements(&self, elements: &mut [DocumentElement], offset: usize) {
        let mut stack: Vec<&mut DocumentElement> = elements.iter_mut().collect();
        while let Some(el) = stack.pop() {
            el.line_start = el.line_start.saturating_add(offset);
            el.line_end = el.line_end.saturating_add(offset);
            stack.extend(el.children.iter_mut());
        }
    }

    fn detect_content_type(&self, el: &DocumentElement) -> DetectionResult {
        // Only look for Markdown in HTML elements for now as per refactor plan
        if el.element_type != ElementType::HtmlElement {
            return DetectionResult::None;
        }

        if let ElementAttributes::Html(ref html) = el.attributes {
            // Check explicit attributes first
            for key in ["class", "lang", "language", "type"] {
                if let Some(serde_json::Value::String(val)) = html.other.get(key) {
                    let val = val.to_lowercase();
                    if val.contains("markdown") || val.contains("md") {
                        return DetectionResult::Detected(FileType::Markdown);
                    }
                }
            }
        }

        // Heuristic fallback
        let preview_len = el.content.len().min(self.heuristic_window);
        let preview = &el.content[..preview_len];
        let trimmed = preview.trim();

        if trimmed.len() > 2 {
            let looks_like_md = trimmed.starts_with("# ") || 
                               trimmed.contains("\n# ") || 
                               trimmed.contains("```") ||
                               trimmed.starts_with("- ") ||
                               trimmed.starts_with("* ");
            
            if looks_like_md {
                 // Check if it's just a single line that might be a false positive (bullet points)
                if !trimmed.contains('\n') && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
                    return DetectionResult::None; 
                }
                return DetectionResult::Detected(FileType::Markdown);
            }
        }

        DetectionResult::None
    }
}
