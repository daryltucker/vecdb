use crate::types::{DocumentElement, ElementType, ParsedDocument, FileType, DocumentMetadata, HtmlAttributes, ElementAttributes};
use crate::parser::Parser;
use crate::error::{VecqResult, VecqError};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct HtmlParser;

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

impl HtmlParser {
    pub fn new() -> Self {
        Self
    }

    pub fn is_void_tag(tag: &str) -> bool {
        let tag = tag.to_lowercase();
        matches!(tag.as_str(), 
            "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | 
            "input" | "link" | "meta" | "param" | "source" | "track" | "wbr"
        )
    }
}

#[async_trait]
impl Parser for HtmlParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        let meta = DocumentMetadata::new(PathBuf::from("content.html"), content.len() as u64)
            .with_file_type(FileType::Html);
        let mut document = ParsedDocument::new(meta);
        parse_html_internal(&mut document, content)?;
        Ok(document)
    }

    fn file_extensions(&self) -> &[&str] {
        &["html", "htm", "xml", "xhtml"]
    }

    fn language_name(&self) -> &str {
        "HTML"
    }
}

fn parse_html_internal(document: &mut ParsedDocument, content: &str) -> VecqResult<()> {
    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = false; 

    // Helper to convert u64 position to usize safely
    let to_usize = |pos: u64| -> usize {
        pos.try_into().unwrap_or(usize::MAX)
    };

    struct OpenElement {
        tag: String,
        start_pos: usize, 
        start_line: usize,
        attributes: HashMap<String, serde_json::Value>,
        children: Vec<DocumentElement>,
        consecutive_count: usize, // For structural validation
    }

    let mut open_stack: Vec<OpenElement> = Vec::new();
    
    let line_counter = crate::parser::utils::LineCounter::new(content);
    let get_line_number = |pos: usize| line_counter.get_line_number(pos);

    let mut buf = Vec::new();

    // Structural constraints (D026)
    const MAX_STRUCTURAL_DEPTH: usize = 200; // Increased from 20 for "Pure HTML" mode
    const MAX_ELEMENTS_PER_FILE: usize = 100000; // Increased for "Pure HTML" mode
    const MAX_MALFORMED_TAGS: usize = 500;
    const CONSECUTIVE_TAG_LIMIT: usize = 100; // Fail if 100 <div><div> in a row with no attributes/content

    let mut malformed_count = 0;

    loop {
        if document.elements.len() > MAX_ELEMENTS_PER_FILE {
            return Err(VecqError::CircuitBreakerTriggered { 
                message: format!("File exceeded maximum element limit of {}", MAX_ELEMENTS_PER_FILE) 
            });
        }

        let event_start = to_usize(reader.buffer_position());
        
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let start_line = get_line_number(event_start);
                
                let mut attributes = HashMap::new();
                for attr in e.attributes().flatten() {
                    let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                    let value = String::from_utf8_lossy(attr.value.as_ref()).to_string();
                    attributes.insert(key, serde_json::Value::String(value));
                }

                let content_start = to_usize(reader.buffer_position()); 

                if HtmlParser::is_void_tag(&tag_name) {
                    let element = DocumentElement::new(
                        ElementType::HtmlElement,
                        Some(tag_name),
                        String::new(),
                        start_line,
                        start_line,
                    ).set_attributes(ElementAttributes::Html(HtmlAttributes {
                        other: attributes,
                    }));
                    if let Some(parent) = open_stack.last_mut() {
                        parent.children.push(element);
                    } else {
                        document.elements.push(element);
                    }
                } else if open_stack.len() < MAX_STRUCTURAL_DEPTH {
                    // D026: Consecutive Tag Check
                    let mut consecutive_count = 0;
                    if let Some(last) = open_stack.last() {
                        if last.tag == tag_name && last.attributes.is_empty() && attributes.is_empty() {
                            consecutive_count = last.consecutive_count + 1;
                            if consecutive_count > CONSECUTIVE_TAG_LIMIT {
                                return Err(VecqError::CircuitBreakerTriggered {
                                    message: format!("Structural Error: Consecutive open tags of type '{}' without content or attributes. Rejecting potential bomb.", tag_name)
                                });
                            }
                        }
                    }

                    open_stack.push(OpenElement {
                        tag: tag_name,
                        start_pos: content_start,
                        start_line,
                        attributes,
                        children: Vec::new(),
                        consecutive_count,
                    });
                } else {
                    malformed_count += 1;
                    if malformed_count > MAX_MALFORMED_TAGS {
                        return Err(VecqError::CircuitBreakerTriggered {
                            message: "Too many malformed or deeply nested tags".to_string()
                        });
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                
                let mut found_idx = None;
                for (i, open) in open_stack.iter().enumerate().rev() {
                    if open.tag == tag_name {
                        found_idx = Some(i);
                        break;
                    }
                }

                if let Some(idx) = found_idx {
                    while open_stack.len() > idx {
                        if let Some(open) = open_stack.pop() {
                            let content_end = event_start; 
                            
                            let safe_start = open.start_pos.min(content.len());
                            let safe_end = content_end.min(content.len());
                            
                            let raw_content = if safe_start <= safe_end {
                                &content[safe_start..safe_end]
                            } else {
                                ""
                            };

                            let end_line = get_line_number(to_usize(reader.buffer_position()));

                            let mut element = DocumentElement::new(
                                ElementType::HtmlElement,
                                Some(open.tag.clone()),
                                raw_content.to_string(),
                                open.start_line,
                                end_line,
                            );
                            
                            element = element.set_attributes(ElementAttributes::Html(HtmlAttributes {
                                other: open.attributes,
                            }));
                            
                            element = element.with_children(open.children);

                            if let Some(parent) = open_stack.last_mut() {
                                parent.children.push(element);
                            } else {
                                document.elements.push(element);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                let line = get_line_number(to_usize(reader.buffer_position()));
                return Err(VecqError::parse_error(
                    document.metadata.path.clone(),
                    line,
                    e.to_string(),
                    Some(e)
                ));
            }
            _ => {}
        }
        buf.clear();
    }
    
    // Handle unclosed tags
    while let Some(open) = open_stack.pop() {
        let end_pos = content.len();
        let safe_start = open.start_pos.min(end_pos);
        let raw_content = &content[safe_start..end_pos];
        let end_line = get_line_number(end_pos);

        let mut element = DocumentElement::new(
            ElementType::HtmlElement,
            Some(open.tag),
            raw_content.to_string(),
            open.start_line,
            end_line,
        );

        element = element.set_attributes(ElementAttributes::Html(HtmlAttributes {
            other: open.attributes,
        }));
        
        element = element.with_children(open.children);

        if let Some(parent) = open_stack.last_mut() {
            parent.children.push(element);
        } else {
            document.elements.push(element);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocumentMetadata, FileType};
    use std::path::PathBuf;

    fn parse_html(content: &str) -> ParsedDocument {
        let meta = DocumentMetadata::new(PathBuf::from("test.html"), content.len() as u64)
            .with_file_type(FileType::Html);
        let mut doc = ParsedDocument::new(meta);
        parse_html_internal(&mut doc, content).unwrap();
        doc
    }

    #[test]
    fn test_simple_element() {
        let doc = parse_html("<div class=\"test\">Hello</div>");
        assert_eq!(doc.elements.len(), 1);
        let el = &doc.elements[0];
        assert_eq!(el.name.as_deref(), Some("div"));
        assert_eq!(el.content, "Hello");
        assert_eq!(el.attributes.get_text("class").unwrap(), "test");
    }

    #[test]
    fn test_nested_elements() {
        let doc = parse_html("<outer><inner>Text</inner></outer>");
        assert_eq!(doc.elements.len(), 1);
        let outer = &doc.elements[0];
        assert_eq!(outer.name.as_deref(), Some("outer"));
        assert_eq!(outer.children.len(), 1);
        let inner = &outer.children[0];
        assert_eq!(inner.name.as_deref(), Some("inner"));
        assert_eq!(inner.content, "Text");
    }

    #[test]
    fn test_mixed_content() {
        let html = "<section>\n## Header\n</section>";
        let doc = parse_html(html);
        let el = &doc.elements[0];
        assert!(el.content.contains("## Header"));
    }
    
    #[test]
    fn test_custom_tags() {
        let doc = parse_html("<mcp_servers>List</mcp_servers>");
        assert_eq!(doc.elements[0].name.as_deref(), Some("mcp_servers"));
    }

    #[test]
    fn test_recursion_safety_performance() {
        let mut deep_html = "Text content\n".to_string();
        for i in 0..150 {
            deep_html = format!("<div id=\"{}\">{}</div>", i, deep_html);
        }
        
        let start = std::time::Instant::now();
        let doc = parse_html(&deep_html);
        let duration = start.elapsed();
        
        assert!(!doc.elements.is_empty());
        // Skeleton-first should be extremely fast, even with 150 levels
        assert!(duration.as_millis() < 50, "Skeleton refactor should be ultra-fast: {:?}", duration);
    }

    #[test]
    fn test_3kb_fixture() {
        let path = std::path::Path::new("tests/fixtures/perf_3kb.html");
        if !path.exists() { return; }
        let content = std::fs::read_to_string(path).unwrap();
        let doc = parse_html(&content);
        assert!(!doc.elements.is_empty());
        assert!(doc.elements.iter().any(|e| e.name.as_deref() == Some("div")));
    }

    #[test]
    fn test_circuit_breaker_skeleton() {
        // Test Element Limit
        let many_tags = "<div></div>".repeat(110000);
        let meta = DocumentMetadata::new(PathBuf::from("bomb.html"), many_tags.len() as u64);
        let mut doc = ParsedDocument::new(meta);
        let result = parse_html_internal(&mut doc, &many_tags);
        assert!(result.is_err());
        match result.unwrap_err() {
            VecqError::CircuitBreakerTriggered { message } => assert!(message.contains("limit")),
            _ => panic!("Expected CircuitBreakerTriggered (Limit)"),
        }

        // Test Consecutive Tag Bomb (D026)
        let consecutive_bomb = "<div>".repeat(102);
        let meta = DocumentMetadata::new(PathBuf::from("deep.html"), consecutive_bomb.len() as u64);
        let mut doc = ParsedDocument::new(meta);
        let result = parse_html_internal(&mut doc, &consecutive_bomb);
        assert!(result.is_err());
        match result.unwrap_err() {
            VecqError::CircuitBreakerTriggered { message } => {
                assert!(message.contains("Consecutive open tags"));
                assert!(message.contains("div"));
            },
            _ => panic!("Expected CircuitBreakerTriggered (Consecutive)"),
        }
    }

    #[test]
    fn test_enrich_markdown() {
        let text = "<task>\n# Header\n- Item 1\n</task>";
        let doc = parse_html(text);
        
        let task = &doc.elements[0];
        // In Skeleton mode, there should be no Markdown children yet
        assert!(task.children.is_empty(), "Skeleton should not have markdown children");
        
        // Enrich it
        let enriched_doc = crate::enrich_document(doc).unwrap();
        let enriched_task = &enriched_doc.elements[0];
        
        assert_eq!(enriched_task.children.len(), 3, "Enriched task should have 3 markdown children (Header, List, and ListItem)");
        let header = &enriched_task.children[0];
        assert_eq!(header.element_type, crate::types::ElementType::Header);
        assert_eq!(header.name.as_deref(), Some("Header"));
        assert_eq!(header.line_start, 2);
    }

    #[test]
    fn test_html_failure_fixture() {
        let path = std::path::Path::new("tests/fixtures/html_failure.html");
        if !path.exists() { return; }
        let content = std::fs::read_to_string(path).unwrap();
        
        // This file contains binary junk. While the parser itself might try to find tags,
        // we want to ensure it doesn't hang or crash.
        let doc = parse_html(&content);
        assert!(!doc.elements.is_empty()); 
    }
}
