use proptest::prelude::*;
use vecq::parser::Parser;
use vecq::parsers::RustParser;
use vecq::types::ElementType;

proptest! {
    #[test]
    fn test_rust_parsing_does_not_crash(s in "\\PC*") {
        let parser = RustParser::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Ignore errors, just check for panic
        let _ = rt.block_on(parser.parse(&s));
    }
}

// Hierarchical tests using specific fixtures
#[tokio::test]
async fn test_rust_hierarchy() {
    let content = r#"
        mod my_mod {
            pub struct MyStruct;
            
            impl MyStruct {
                pub fn method() {}
            }
        }
    "#;

    let parser = RustParser::new();
    let doc = parser.parse(content).await.unwrap();

    // Check root module
    let modules = doc.find_elements(ElementType::Module);
    assert_eq!(modules.len(), 1);
    let my_mod = modules[0].clone(); // Clone to avoid borrow issues if needed, or index
    assert_eq!(my_mod.name.as_deref(), Some("my_mod")); // Tree-sitter might have different structure/nesting?

    // Check struct
    let structs = doc.find_elements(ElementType::Struct);
    assert_eq!(structs.len(), 1);
    assert_eq!(structs[0].name.as_deref(), Some("MyStruct"));

    // Check impl
    let impls = doc.find_elements(ElementType::Implementation);
    assert_eq!(impls.len(), 1);

    // Check method
    let methods = doc.find_elements(ElementType::Function);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name.as_deref(), Some("method"));
}
