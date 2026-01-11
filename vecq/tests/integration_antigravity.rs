use vecq::parsers::create_parser;
use vecq::types::{FileType, ElementType};

#[tokio::test]
async fn test_antigravity_prompt_parsing() {
    let content = r#"
<system>
    <role>Assistant</role>
</system>
<task>
# Task Title

1. Step one
2. Step two
</task>
"#;

    let parser = create_parser(FileType::Html).expect("Failed to create parser");
    let doc = parser.parse(content).await.expect("Failed to parse");
    // D026: Enrich the document to detect and parse Markdown in <task>
    let doc = vecq::enrich_document(doc).expect("Failed to enrich");
    
    // Check root elements
    let system = doc.elements.iter().find(|e| e.name.as_deref() == Some("system")).expect("system tag not found");
    // XML parser streaming: children are in e.children
    let role = system.children.iter().find(|e| e.name.as_deref() == Some("role")).expect("role tag not found");
    assert_eq!(role.content, "Assistant");
    
    let task = doc.elements.iter().find(|e| e.name.as_deref() == Some("task")).expect("task tag not found");
    
    // Check content detection
    assert_eq!(task.attributes.get("x-detected-content-type").and_then(|v| v.as_str()), Some("Markdown"));
    
    // Check recursive parsing results
    // We expect a Header and a List
    // In markdown parser, ElementType::Header name is the header text.
    let header = task.children.iter().find(|e| e.element_type == ElementType::Header);
    
    assert!(header.is_some(), "Markdown Header not found in task children");
    assert_eq!(header.unwrap().name.as_deref(), Some("Task Title"));
    
    let list = task.children.iter().find(|e| e.element_type == ElementType::List);
    assert!(list.is_some(), "Markdown List not found in task children");
}
