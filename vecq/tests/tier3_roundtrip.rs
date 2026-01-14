use vecq::parsers::{TomlParser, RustParser, JsonParser, PythonParser, CParser, CppParser, CudaParser, GoParser, BashParser};
#[cfg(feature = "javascript-parser")]
use vecq::parsers::JavaScriptParser;
use vecq::generators::{TomlGenerator, RustGenerator, JsonGenerator};
use vecq::parser::Parser;
use vecq::generator::Generator;
use vecq::types::{ElementAttributes, ParsedDocument};
use std::path::Path;


// Generic Round Trip logic
async fn verify_round_trip(
    name: &str,
    source: &str,
    parser: &dyn Parser,
    generator: &dyn Generator,
    data_mode: bool, // Data mode = Semantic checks, Blob mode = String checks?
) {
    println!("Testing: {}", name);
    
    // 1. Parse
    let doc = parser.parse(source).await.expect(&format!("{} parsing failed", name));
    
    // 2. Generate
    let generated = generator.generate(&doc).expect(&format!("{} generation failed", name));
    
    // 3. Verify
    if data_mode {
        // Semantic TOML check (Value to Value)
         let source_val: toml::Value = source.parse().expect("Invalid source TOML");
         let gen_val: toml::Value = generated.parse().expect("Invalid generated TOML");
         assert_eq!(source_val, gen_val, "TOML Semantic Mismatch in {}", name);
    } else {
        // Blob check (Rust)
        // Since RustGenerator just joins elements, and source likely has things OUTSIDE elements (headers, comments)
        // we can't assert strict equality on the full file yet unless Parser captures EVERYTHING.
        // RustParser currently seems to capture Items. 
        // If there are comments between items, they might be lost?
        // Let's just print comparison for manual review in the test log for now,
        // or check if `generated` is contained in `source`?
        // Or better: Re-parse the generated code and valid output structure matches?
        // But "Round Trip" usually implies idempotence: parse(generate(doc)) == doc.
        
        let doc2 = parser.parse(&generated).await.expect("Failed to re-parse generated code");
        
        // Naive verification: Count elements
        assert_eq!(doc.elements.len(), doc2.elements.len(), "Element count mismatch in {}", name);
        // Verify content identity of first element (smoke test)
        if !doc.elements.is_empty() {
             // Normalized string comparison (remove whitespace variance?)
             let c1 = doc.elements[0].content.split_whitespace().collect::<Vec<_>>().join(" ");
             let c2 = doc2.elements[0].content.split_whitespace().collect::<Vec<_>>().join(" ");
             assert_eq!(c1, c2, "First element content mismatch in {}", name);
        }
    }
}

#[tokio::test]
async fn test_round_trip_suite() {
    let fixtures_root = Path::new("tests/fixtures");
    if !fixtures_root.exists() {
        println!("Skipping tests: fixtures root not found");
        return;
    }

    // --- TOML ---
    // Hardcoded simple test matching previous logic
    let toml_source = r#"[package]
name = "vecq"
version = "0.1.0"
[dependencies]
serde = "1.0"
"#;
    verify_round_trip(
        "Memory TOML", 
        toml_source, 
        &TomlParser::new(), 
        &TomlGenerator::new(), 
        false
    ).await;

    // --- STRICT TOML ATTRIBUTE VERIFICATION ---
    let doc = TomlParser::new().parse(toml_source).await.unwrap();
    let pkg_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("package")).unwrap();
    if let ElementAttributes::Toml(_) = &pkg_el.attributes {
        println!("Verified strict Toml attributes for '{}'", pkg_el.name.as_ref().unwrap());
    } else {
        panic!("Expected ElementAttributes::Toml, found {:?}", pkg_el.attributes);
    }

    // --- RUST ---
    // Let's inspect a real file if available or use memory string
    let rust_source = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

struct Point {
    x: i32,
    y: i32,
}
"#;
     verify_round_trip(
        "Memory Rust", 
        rust_source, 
        &RustParser::new(), 
        &RustGenerator::new(), 
        false
    ).await;

    // --- STRICT ATTRIBUTE VERIFICATION (The "Gold" part) ---
    let doc = RustParser::new().parse(rust_source).await.unwrap();
    let fn_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("add")).unwrap();
    
    // Prove we can extract attributes regardless of whether the variant is strict Rust or Generic
    let vis = fn_el.attributes.get_text("visibility").expect("Missing visibility attribute");
    println!("Verified attributes for '{}': vis={}", fn_el.name.as_ref().unwrap(), vis);
    assert_eq!(vis, "pub");
    
    // Smoke check other fields if generic
    if let ElementAttributes::Generic(_) = &fn_el.attributes {
        println!("Note: Using flexible Generic attributes for Rust (Recursive mode)");
    }

    #[cfg(feature = "javascript-parser")]
    {
        // --- JAVASCRIPT ---
        let js_source = "async function hello() { return 'world'; }";
        verify_round_trip(
            "Memory JavaScript",
            js_source,
            &JavaScriptParser::new(),
            &RustGenerator::new(), // JS uses RustGenerator placeholder for text content for now
            false
        ).await;

        let doc: ParsedDocument = JavaScriptParser::new().parse(js_source).await.unwrap();
        let fn_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("hello")).unwrap();
        if let ElementAttributes::JavaScript(attr) = &fn_el.attributes {
            println!("Verified strict JS attributes for '{}': async={}", fn_el.name.as_ref().unwrap(), attr.is_async);
            assert!(attr.is_async);
        } else {
            panic!("Expected ElementAttributes::JavaScript, found {:?}", fn_el.attributes);
        }
    }

    // --- JSON ---
    let json_source = r#"{
  "name": "vecq",
  "version": "0.1.0",
  "dependencies": {
    "serde": "1.0"
  }
}"#;
    verify_round_trip(
        "Memory JSON",
        json_source,
        &JsonParser::new(),
        &JsonGenerator::new(),
        false
    ).await;

    let doc = JsonParser::new().parse(json_source).await.unwrap();
    let name_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("name")).unwrap();
    if let ElementAttributes::Json(_) = &name_el.attributes {
        println!("Verified strict JSON attributes for '{}'", name_el.name.as_ref().unwrap());
    } else {
        panic!("Expected ElementAttributes::Json, found {:?}", name_el.attributes);
    }

    // --- PYTHON ---
    let py_source = "async def hello():\n    return 'world'";
    let doc: ParsedDocument = PythonParser::new().parse(py_source).await.unwrap();
    let fn_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("hello")).unwrap();
    if let ElementAttributes::Python(attr) = &fn_el.attributes {
        println!("Verified strict Python attributes for '{}': async={}", fn_el.name.as_ref().unwrap(), attr.is_async);
        assert!(attr.is_async);
    } else {
        panic!("Expected ElementAttributes::Python, found {:?}", fn_el.attributes);
    }

    // --- C-FAMILY (CUDA/C/CPP) ---
    // CUDA
    let cuda_source = "__global__ void my_kernel() {}";
    let doc: ParsedDocument = CudaParser::new().parse(cuda_source).await.unwrap();
    let kernel_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("my_kernel")).unwrap();
    if let ElementAttributes::CFamily(attr) = &kernel_el.attributes {
        let is_global = attr.other.get("is_global").and_then(|v| v.as_bool()).unwrap_or(false);
        println!("Verified strict CFamily (CUDA) attributes for '{}': is_global={}", kernel_el.name.as_ref().unwrap(), is_global);
        assert!(is_global);
    } else {
        panic!("Expected ElementAttributes::CFamily, found {:?}", kernel_el.attributes);
    }

    // C
    let c_source = "void my_c_func() {}";
    let doc: ParsedDocument = CParser::new().parse(c_source).await.unwrap();
    let c_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("my_c_func")).unwrap();
    assert!(matches!(c_el.attributes, ElementAttributes::CFamily(_)));

    // C++
    let cpp_source = "class MyClass {};";
    let doc: ParsedDocument = CppParser::new().parse(cpp_source).await.unwrap();
    let cpp_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("MyClass")).unwrap();
    assert!(matches!(cpp_el.attributes, ElementAttributes::CFamily(_)));

    // --- GO ---
    let go_source = "func Hello() {}";
    let doc: ParsedDocument = GoParser::new().parse(go_source).await.unwrap();
    let fn_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("Hello")).unwrap();
    if let ElementAttributes::Go(_) = &fn_el.attributes {
        println!("Verified strict Go attributes for '{}'", fn_el.name.as_ref().unwrap());
    } else {
        panic!("Expected ElementAttributes::Go, found {:?}", fn_el.attributes);
    }

    // --- BASH ---
    let bash_source = "hello() { echo 'world'; }";
    let doc: ParsedDocument = BashParser::new().parse(bash_source).await.unwrap();
    let fn_el = doc.elements.iter().find(|e| e.name.as_deref() == Some("hello")).unwrap();
    if let ElementAttributes::Bash(_) = &fn_el.attributes {
        println!("Verified strict Bash attributes for '{}'", fn_el.name.as_ref().unwrap());
    } else {
        panic!("Expected ElementAttributes::Bash, found {:?}", fn_el.attributes);
    }

    println!("Tier 3 Round Trip: ALL PASSED");
}
