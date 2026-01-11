use vecq::parsers::{RustParser, RustTreeSitterParser};
use vecq::parser::Parser;
use vecq::types::{DocumentElement, ElementType};
use pretty_assertions::assert_eq;

// THE GOLDEN MASTER CONTRACT
// This test ensures that the new Tree-sitter parser produces the exact same
// logical output as the legacy Syn parser for a complex Rust file.
//
// If this test fails, it means we are breaking the "Canonical Schema"
// and potentially breaking downstream tools (docs, agents).

#[tokio::test]
async fn test_schema_contract_boids() {
    // Helper to normalize signatures for comparison (Aggressive: Strip all whitespace)
    fn normalize_sig(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    let boids_content = include_str!("../../demo/algorithms/boids.rs");
    
    // 1. Parse with Legacy (Syn) - The "Golden Standard"
    let legacy_parser = RustParser::new();
    let legacy_doc = legacy_parser.parse(boids_content).await.expect("Legacy parser failed");
    
    // 2. Parse with New (Tree-sitter) - The "Candidate"
    let new_parser = RustTreeSitterParser::new();
    let new_doc = new_parser.parse(boids_content).await.expect("New parser failed");

    // 3. Compare Critical Structures
    // We don't compare line numbers exactly yet if they differ slightly by design,
    // but we MUST match signatures, names, and docstrings.
    
    let legacy_funcs = extract_functions(&legacy_doc.elements);
    let new_funcs = extract_functions(&new_doc.elements);
    
    // Check that we found the same number of functions
    assert_eq!(
        legacy_funcs.len(), 
        new_funcs.len(), 
        "Function count mismatch! Legacy found {}, New found {}", 
        legacy_funcs.len(), 
        new_funcs.len()
    );

    // Check each function matches
    for (i, (l_func, n_func)) in legacy_funcs.iter().zip(new_funcs.iter()).enumerate() {
        assert_eq!(
            l_func.name, 
            n_func.name, 
            "Function name mismatch at index {}", i
        );
        
        // Detailed check on docstrings (The "Doc" part of vecq doc)
        // Normalize docstrings (trim) to be safe
        let l_doc = l_func.attributes.get("docstring").and_then(|v| v.as_str()).map(|s| s.trim());
        let n_doc = n_func.attributes.get("docstring").and_then(|v| v.as_str()).map(|s| s.trim());
        
        assert_eq!(
            l_doc, 
            n_doc, 
            "Docstring mismatch for function {:?}", l_func.name
        );
        
         // Detailed check on signatures
        let l_sig = l_func.attributes.get("signature").and_then(|v| v.as_str()).map(normalize_sig);
        let n_sig = n_func.attributes.get("signature").and_then(|v| v.as_str()).map(normalize_sig);

         assert_eq!(
            l_sig,
            n_sig,
            "Signature mismatch for function {:?}", l_func.name
        );
    }
}

// Helper to extract just the function elements for comparison
fn extract_functions(elements: &[DocumentElement]) -> Vec<&DocumentElement> {
    elements.iter()
        .filter(|e| e.element_type == ElementType::Function)
        .collect()
}
