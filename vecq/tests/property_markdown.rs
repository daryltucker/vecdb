use proptest::prelude::*;
use vecq::parser::Parser;
use vecq::parsers::MarkdownParser;
use vecq::types::ElementType;

proptest! {
    #[test]
    fn test_markdown_parsing_does_not_crash(s in "\\PC*") {
        let parser = MarkdownParser::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(parser.parse(&s));
    }

    #[test]
    fn test_markdown_header_extraction(
        level in 1..=6i32,
        title in "[a-zA-Z0-9 ]+"
    ) {
        let content = format!("{} {}\n", "#".repeat(level as usize), title);
        let parser = MarkdownParser::new();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let doc = rt.block_on(parser.parse(&content)).unwrap();
        let headers = doc.find_elements(ElementType::Header);

        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].name.as_ref().unwrap(), title.trim());

        // Check level attribute
        let level_attr = headers[0].attributes.get("level").unwrap();
        assert_eq!(level_attr.as_i64(), Some(level as i64));
    }

    #[test]
    fn test_markdown_codeblock_extraction(
        lang in "[a-z]+",
        code in "[a-zA-Z0-9 =;]+"
    ) {
        let content = format!("```{}\n{}\n```\n", lang, code);
        let parser = MarkdownParser::new();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let doc = rt.block_on(parser.parse(&content)).unwrap();
        let blocks = doc.find_elements(ElementType::CodeBlock);

        assert!(!blocks.is_empty());
        let block = blocks[0];

        assert!(block.content.contains(&code));
        let lang_attr = block.attributes.get("language").unwrap();
        assert_eq!(lang_attr.as_str(), Some(lang.as_str()));
    }
}
