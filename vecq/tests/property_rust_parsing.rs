use vecq::parsers::RustParser;
use vecq::parser::Parser;
use vecq::types::ElementType;
use proptest::prelude::*;

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
    let my_mod = modules[0];
    assert_eq!(my_mod.name, Some("my_mod".to_string()));
    
    // Check struct inside module
    let structs = my_mod.find_children(ElementType::Struct);
    assert_eq!(structs.len(), 1);
    assert_eq!(structs[0].name, Some("MyStruct".to_string()));
    
    // Check impl inside module
    let impls = my_mod.find_children(ElementType::Implementation);
    assert_eq!(impls.len(), 1);
    
    // Check method inside impl
    let methods = impls[0].find_children(ElementType::Function);
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, Some("method".to_string()));
}
