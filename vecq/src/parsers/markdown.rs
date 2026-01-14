use crate::error::VecqResult;
use crate::parser::Parser;
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument};
use async_trait::async_trait;
use pulldown_cmark::{Event, HeadingLevel, Options, Tag, TagEnd};
use std::path::PathBuf;

#[derive(Clone)]
pub struct MarkdownParser;

impl Default for MarkdownParser {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownParser {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Parser for MarkdownParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
       Ok(parse_markdown_document(content))
    }

    fn file_extensions(&self) -> &[&str] {
        &["md", "markdown"]
    }

    fn language_name(&self) -> &str {
        "Markdown"
    }
}

pub fn parse_markdown_document(content: &str) -> ParsedDocument {
     let metadata = DocumentMetadata::new(PathBuf::from(""), content.len() as u64)
            .with_line_count(content)
            .with_file_type(FileType::Markdown);
     let mut doc = ParsedDocument::new(metadata);
     
     let mut options = Options::empty();
     options.insert(Options::ENABLE_TABLES);
     options.insert(Options::ENABLE_TASKLISTS);
     options.insert(Options::ENABLE_STRIKETHROUGH);
     options.insert(Options::ENABLE_FOOTNOTES);
     
     let parser = pulldown_cmark::Parser::new_ext(content, options);
     let events = parser.into_offset_iter();
     
     let line_counter = crate::parser::utils::LineCounter::new(content);
     let get_line_number = |pos: usize| line_counter.get_line_number(pos);

     // State tracking
     let mut in_header = false;
     let mut header_level = 0;
     let mut header_start = 0;
     let mut header_text = String::new();
     let mut codeblock_lang = String::new();
     let mut codeblock_start = 0;
     let mut in_paragraph = false;
     let mut paragraph_start = 0;
     let mut paragraph_text = String::new();
     let mut in_blockquote = false;
     let mut blockquote_start = 0;
     let mut blockquote_text = String::new();
     let mut list_start = 0;
     let mut list_ordered = false;
     let mut table_start = 0;

     // Inline style tracking
     // We need stacks because they can nest (e.g. bold inside italic)
     // BUT, vecq element model is somewhat flat for "find all bold spans".
     // We'll track the start positions.
     let mut emphasis_start: Vec<usize> = Vec::new();
     let mut strong_start: Vec<usize> = Vec::new();
     let mut strikethrough_start: Vec<usize> = Vec::new();
     
     // List Item tracking
     let mut item_start: Vec<usize> = Vec::new();
     // We need to track if we saw a task list marker for the *current* item
     let mut current_item_task_status: Option<bool> = None; 
     
     // Footnotes
     let mut footnote_def_start = 0;
     let mut footnote_name = String::new();

     // Link state
     let mut in_link = false;
     let mut link_start = 0;
     let mut link_url = String::new();
     let mut link_title = String::new();
     let mut link_text = String::new();
     
     for (event, range) in events {
        match event {
            // --- Structural Blocks ---
            Event::Start(Tag::Heading { level, .. }) => {
                in_header = true;
                header_level = match level {
                    HeadingLevel::H1 => 1, HeadingLevel::H2 => 2, HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4, HeadingLevel::H5 => 5, HeadingLevel::H6 => 6,
                };
                header_start = range.start;
                header_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_header = false;
                let start_line = get_line_number(header_start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::Header,
                    Some(header_text.trim().to_string()),
                    content[header_start..range.end].to_string(),
                    start_line,
                    end_line,
                ).with_attribute("level".to_string(), header_level);
                
                doc = doc.add_element(element);
            }
            Event::Start(Tag::Paragraph) => {
                in_paragraph = true;
                paragraph_start = range.start;
                paragraph_text.clear();
            }
            Event::End(TagEnd::Paragraph) => {
                in_paragraph = false;
                let start_line = get_line_number(paragraph_start);
                let end_line = get_line_number(range.end);
                
                // If inside a footnote definition, this paragraph is child content, but we verify top-level
                // element creation logic.
                
                let element = DocumentElement::new(
                    ElementType::Paragraph,
                    None,
                    paragraph_text.trim().to_string(),
                    start_line,
                    end_line,
                );
                
                doc = doc.add_element(element);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                in_blockquote = true;
                blockquote_start = range.start;
                blockquote_text.clear();
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                in_blockquote = false;
                let start_line = get_line_number(blockquote_start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::Blockquote,
                    None,
                    content[blockquote_start..range.end].to_string(),
                    start_line,
                    end_line,
                );
                
                doc = doc.add_element(element);
            }
            
            // --- Lists & Items ---
            Event::Start(Tag::List(first_item)) => {
                list_start = range.start;
                list_ordered = first_item.is_some();
            }
            Event::End(TagEnd::List(_)) => {
                let start_line = get_line_number(list_start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::List,
                    None,
                    content[list_start..range.end].to_string(),
                    start_line,
                    end_line,
                ).with_attribute("ordered".to_string(), list_ordered);
                
                doc = doc.add_element(element);
            }
            Event::Start(Tag::Item) => {
                item_start.push(range.start);
                current_item_task_status = None;
            }
            Event::End(TagEnd::Item) => {
                if let Some(start) = item_start.pop() {
                    let start_line = get_line_number(start);
                    let end_line = get_line_number(range.end);
                    
                    let mut element = DocumentElement::new(
                        ElementType::ListItem,
                        None,
                        content[start..range.end].to_string(),
                        start_line,
                        end_line,
                    );
                    
                    if let Some(checked) = current_item_task_status {
                        element = element
                            .with_attribute("task".to_string(), true)
                            .with_attribute("checked".to_string(), checked);
                    }
                    
                    doc = doc.add_element(element);
                }
            }
            Event::TaskListMarker(checked) => {
                current_item_task_status = Some(checked);
            }
            
            // --- Tables ---
            Event::Start(Tag::Table(_)) => {
                table_start = range.start;
            }
            Event::End(TagEnd::Table) => {
                let start_line = get_line_number(table_start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::Table,
                    None,
                    content[table_start..range.end].to_string(),
                    start_line,
                    end_line,
                );
                
                doc = doc.add_element(element);
            }
            
            // --- Media & Formatting ---
            Event::Start(Tag::Image { dest_url, title, .. }) => {
                let start_line = get_line_number(range.start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::Image,
                    Some(title.to_string()),
                    dest_url.to_string(),
                    start_line,
                    end_line,
                );
                
                doc = doc.add_element(element);
            }
             Event::Start(Tag::Link { dest_url, title, .. }) => {
                in_link = true;
                link_start = range.start;
                link_url = dest_url.to_string();
                link_title = title.to_string();
                link_text.clear();
            }
            Event::End(TagEnd::Link) => {
                in_link = false;
                let start_line = get_line_number(link_start);
                let end_line = get_line_number(range.end);
                
                let mut element = DocumentElement::new(
                    ElementType::Link,
                    Some(link_text.trim().to_string()),
                    link_url.clone(), 
                    start_line,
                    end_line,
                );

                if !link_title.is_empty() {
                    element = element.with_attribute("title".to_string(), link_title.clone());
                }
                
                doc = doc.add_element(element);
            }
            
            // --- Styles: Emphasis, Strong, Strikethrough ---
            Event::Start(Tag::Emphasis) => {
                emphasis_start.push(range.start);
            }
            Event::End(TagEnd::Emphasis) => {
                if let Some(start) = emphasis_start.pop() {
                     let start_line = get_line_number(start);
                     let end_line = get_line_number(range.end);
                     let text = content[start..range.end].to_string();
                     // strip markers * or _ roughly? content gives full range including markers
                     // Usually we want the content inside? 
                     // pulldown_cmark range includes markers.
                     // The text inside would be valuable but raw content is safer.
                     
                     let element = DocumentElement::new(
                        ElementType::Emphasis,
                        None,
                        text,
                        start_line,
                        end_line,
                    );
                    doc = doc.add_element(element);
                }
            }
            Event::Start(Tag::Strong) => {
                strong_start.push(range.start);
            }
            Event::End(TagEnd::Strong) => {
                if let Some(start) = strong_start.pop() {
                     let start_line = get_line_number(start);
                     let end_line = get_line_number(range.end);
                     
                     let element = DocumentElement::new(
                        ElementType::Strong,
                        None,
                        content[start..range.end].to_string(),
                        start_line,
                        end_line,
                    );
                    doc = doc.add_element(element);
                }
            }
             Event::Start(Tag::Strikethrough) => {
                strikethrough_start.push(range.start);
            }
            Event::End(TagEnd::Strikethrough) => {
                if let Some(start) = strikethrough_start.pop() {
                     let start_line = get_line_number(start);
                     let end_line = get_line_number(range.end);
                     
                     let element = DocumentElement::new(
                        ElementType::Strikethrough,
                        None,
                        content[start..range.end].to_string(),
                        start_line,
                        end_line,
                    );
                    doc = doc.add_element(element);
                }
            }
            
            // --- Footnotes ---
            Event::Start(Tag::FootnoteDefinition(name)) => {
                footnote_def_start = range.start;
                footnote_name = name.to_string();
            }
            Event::End(TagEnd::FootnoteDefinition) => {
                let start_line = get_line_number(footnote_def_start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::FootnoteDefinition,
                    Some(footnote_name.clone()),
                    content[footnote_def_start..range.end].to_string(),
                    start_line,
                    end_line,
                );
                doc = doc.add_element(element);
            }
            
            // --- Misc ---
            Event::Rule => {
                let start_line = get_line_number(range.start);
                let end_line = get_line_number(range.end);
                
                let element = DocumentElement::new(
                    ElementType::HorizontalRule,
                    None,
                    content[range.clone()].to_string(),
                    start_line,
                    end_line,
                );
                
                doc = doc.add_element(element);
            }
            Event::Text(text) => {
                if in_link {
                    link_text.push_str(&text);
                }
                
                if in_header {
                    header_text.push_str(&text);
                } else if in_paragraph {
                    paragraph_text.push_str(&text);
                } else if in_blockquote {
                    blockquote_text.push_str(&text);
                }
            }
            Event::Code(text) => {
                let code_text = text.to_string(); 
                
                if in_link {
                    link_text.push_str(&code_text);
                }
                
                if in_header {
                    header_text.push_str(&code_text);
                } else if in_paragraph {
                    paragraph_text.push_str(&code_text);
                } else if in_blockquote {
                    blockquote_text.push_str(&code_text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                let separator = " ";
                if in_link {
                    link_text.push_str(separator);
                }
                if in_header {
                    header_text.push_str(separator);
                } else if in_paragraph {
                    paragraph_text.push_str(separator);
                } else if in_blockquote {
                    blockquote_text.push_str(separator);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                codeblock_start = range.start;
                codeblock_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(l) => l.to_string(),
                    pulldown_cmark::CodeBlockKind::Indented => "indented".to_string(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                 let start_line = get_line_number(codeblock_start);
                 let end_line = get_line_number(range.end);
                 
                 let element = DocumentElement::new(
                     ElementType::CodeBlock,
                     None,
                     content[codeblock_start..range.end].to_string(),
                     start_line,
                     end_line,
                 ).with_attribute("language".to_string(), codeblock_lang.clone());
                 
                 doc = doc.add_element(element);
            }
            _ => {}
        }
     }
     
     doc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_md(content: &str) -> ParsedDocument {
        parse_markdown_document(content)
    }

    #[test]
    fn test_header_parsing() {
        let doc = parse_md("# H1\n## H2\n### H3");
        let headers: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Header)
            .collect();
        assert_eq!(headers.len(), 3);
        assert_eq!(headers[0].name, Some("H1".to_string()));
        assert_eq!(headers[1].name, Some("H2".to_string()));
        assert_eq!(headers[2].name, Some("H3".to_string()));
    }

    #[test]
    fn test_paragraph_parsing() {
        let doc = parse_md("# Header\n\nThis is a paragraph.");
        let paragraphs: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Paragraph)
            .collect();
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].content, "This is a paragraph.");
    }

    #[test]
    fn test_blockquote_parsing() {
        let doc = parse_md("> Quoted text");
        let blockquotes: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Blockquote)
            .collect();
        assert_eq!(blockquotes.len(), 1);
        assert!(blockquotes[0].content.contains("Quoted text"));
    }

    #[test]
    fn test_list_parsing() {
        let doc = parse_md("- Item 1\n- Item 2");
        let lists: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::List)
            .collect();
        assert_eq!(lists.len(), 1);
        assert!(lists[0].content.contains("Item 1"));
    }

    #[test]
    fn test_horizontal_rule_parsing() {
        let doc = parse_md("---\n***\n___");
        let rules: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::HorizontalRule)
            .collect();
        assert_eq!(rules.len(), 3);
    }

    #[test]
    fn test_3kb_fixture() {
        let path = std::path::Path::new("tests/fixtures/perf_3kb.md");
        if !path.exists() { return; }
        let content = std::fs::read_to_string(path).unwrap();
        let doc = parse_markdown_document(&content);
        assert!(!doc.elements.is_empty());
        // Markdown should have headers or paragraphs
        assert!(doc.elements.iter().any(|e| e.element_type == ElementType::Header));
    }
    #[test]
    fn test_code_block_parsing() {
        let doc = parse_md("```rust\nfn main() {}\n```");
        let blocks: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::CodeBlock)
            .collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].attributes.get("language").and_then(|v| v.as_str()), Some("rust"));
    }

    #[test]
    fn test_link_parsing() {
        let doc = parse_md("[Link Text](https://example.com)");
        let links: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Link)
            .collect();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].content, "https://example.com");
    }

    #[test]
    fn test_image_parsing() {
        let doc = parse_md("![Alt](image.png)");
        let images: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Image)
            .collect();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].content, "image.png");
    }

    #[test]
    fn test_table_parsing() {
        let doc = parse_md("| A | B |\n|---|---|\n| 1 | 2 |");
        let tables: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Table)
            .collect();
        assert_eq!(tables.len(), 1);
    }

    #[test]
    fn test_antigravity_chat_format() {
        // Simulates the format from antigravity_chat.jq
        let content = "### 2024-01-01T12:00:00Z\n\nMessage content here.\n\n---\n\n### 2024-01-01T12:01:00Z\n\nAnother message.\n\n---";
        let doc = parse_md(content);
        
        let headers: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Header)
            .collect();
        let paragraphs: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::Paragraph)
            .collect();
        let rules: Vec<_> = doc.elements.iter()
            .filter(|e| e.element_type == ElementType::HorizontalRule)
            .collect();
        
        assert_eq!(headers.len(), 2, "Should have 2 headers (timestamps)");
        assert_eq!(paragraphs.len(), 2, "Should have 2 paragraphs (messages)");
        assert_eq!(rules.len(), 2, "Should have 2 horizontal rules (separators)");
    }
}