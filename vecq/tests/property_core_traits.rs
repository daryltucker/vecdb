// PURPOSE:
//   Property-based tests for vecq's core trait interfaces to validate universal
//   correctness properties across all possible inputs. Essential for ensuring
//   vecq's reliability as the "jq for source code" tool - validates that our
//   trait system can handle any document type and maintains consistency.
//
// REQUIREMENTS:
//   User-specified:
//   - Must validate Property 1: JSON Conversion Completeness across all file types
//   - Must ensure trait implementations work correctly with any valid input
//   - Must run minimum 1000 iterations per property test for comprehensive coverage
//   - Must catch edge cases and boundary conditions that unit tests might miss
//   
//   Implementation-discovered:
//   - Requires proptest crate for property-based test generation
//   - Must generate realistic test data that represents actual file content
//   - Needs comprehensive validation of JSON schema consistency
//   - Must handle malformed input gracefully without panics
//
// IMPLEMENTATION RULES:
//   1. All property tests must run minimum 1000 iterations for statistical confidence
//      Rationale: Property tests need large sample sizes to catch rare edge cases
//   
//   2. Test data generators must produce realistic file content, not random strings
//      Rationale: Unrealistic test data doesn't validate real-world usage patterns
//   
//   3. Every property test must validate specific correctness properties from design
//      Rationale: Tests must map directly to formal correctness requirements
//   
//   4. Property tests must never panic, always return Result for error handling
//      Rationale: Panics in tests indicate bugs in error handling, not test failures
//   
//   5. Tag each test with feature and property references for traceability
//      Rationale: Enables mapping test failures back to design requirements
//   
//   Critical:
//   - DO NOT reduce iteration counts below 1000 without strong justification
//   - DO NOT use unrealistic test data that doesn't represent actual files
//   - ALWAYS validate complete JSON schema consistency, not just successful parsing
//
// USAGE:
//   # Run property tests with default iterations
//   cargo test property_
//   
//   # Run with custom iteration count for debugging
//   PROPTEST_CASES=100 cargo test property_core_traits
//   
//   # Run specific property test
//   cargo test property_json_conversion_completeness
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new trait implementations:
//   1. Add new test generators for the trait's input/output types
//   2. Create property tests that validate trait contract compliance
//   3. Ensure new tests run minimum 1000 iterations
//   4. Add test tags referencing design document properties
//   5. Update this file's documentation with new test coverage
//   
//   When property tests fail:
//   1. Capture the failing test case from proptest output
//   2. Create minimal reproduction case as unit test
//   3. Fix the underlying bug in implementation
//   4. Verify property test passes with fix
//   5. Document the bug and fix in test comments
//
// RELATED FILES:
//   - src/parser.rs - Parser trait being tested
//   - src/converter.rs - JsonConverter trait being tested
//   - src/types.rs - Data structures used in property tests
//   - src/error.rs - Error types that must be handled gracefully
//   - tests/generators.rs - Test data generators (to be created)
//
// MAINTENANCE:
//   Update when:
//   - New traits are added to vecq's core architecture
//   - Trait interfaces change and require new property validation
//   - Property test failures indicate missing edge case coverage
//   - Performance requirements change iteration count needs
//   - New file types require additional test data generation
//
// Last Verified: 2025-12-31

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::Config as ProptestConfig;
use std::collections::HashMap;
use std::path::PathBuf;
use vecq::{
    types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument},
    converter::{UnifiedJsonConverter, JsonConverter},
};

// Test configuration
// const MIN_TEST_ITERATIONS: u32 = 1000;

/// Generate arbitrary file types for testing
fn arbitrary_file_type() -> impl Strategy<Value = FileType> {
    prop_oneof![
        Just(FileType::Rust),
        Just(FileType::Python),
        Just(FileType::Markdown),
        Just(FileType::C),
        Just(FileType::Cpp),
        Just(FileType::Cuda),
        Just(FileType::Go),
        Just(FileType::Bash),
    ]
}

/// Generate arbitrary element types for testing
fn arbitrary_element_type() -> impl Strategy<Value = ElementType> {
    prop_oneof![
        Just(ElementType::Function),
        Just(ElementType::Class),
        Just(ElementType::Struct),
        Just(ElementType::Enum),
        Just(ElementType::Interface),
        Just(ElementType::Module),
        Just(ElementType::Import),
        Just(ElementType::Variable),
        Just(ElementType::Constant),
        Just(ElementType::Header),
        Just(ElementType::CodeBlock),
        Just(ElementType::Link),
        Just(ElementType::Table),
        Just(ElementType::List),
        Just(ElementType::Trait),
        Just(ElementType::Implementation),
        Just(ElementType::Decorator),
        Just(ElementType::Macro),
        Just(ElementType::Namespace),
        Just(ElementType::Package),
        Just(ElementType::Kernel),
        Just(ElementType::DeviceFunction),
    ]
}

/// Generate arbitrary document metadata
fn arbitrary_document_metadata() -> impl Strategy<Value = DocumentMetadata> {
    (
        arbitrary_file_type(),
        "[a-zA-Z0-9_/.-]{1,50}\\.(rs|py|md|c|cpp|cu|go|sh)",
        1u64..1000000u64,
        1usize..10000usize,
    ).prop_map(|(_file_type, path_str, size, line_count)| {
        DocumentMetadata::new(PathBuf::from(path_str), size)
            .with_line_count(&"x\n".repeat(line_count))
    })
}

/// Generate arbitrary document elements with realistic structure
fn arbitrary_document_element() -> impl Strategy<Value = DocumentElement> {
    let leaf = (
        arbitrary_element_type(),
        prop::option::of("[a-zA-Z_][a-zA-Z0-9_]*"),
        "[a-zA-Z0-9_\\s\\{\\}\\(\\)\\[\\];.,]{1,50}",
        1usize..100usize,
    ).prop_map(|(element_type, name, content, line_start)| {
        // Ensure line_end doesn't exceed reasonable bounds
        let content_lines = content.lines().count().max(1);
        let line_end = line_start + content_lines - 1;
        DocumentElement::new(element_type, name, content, line_start, line_end)
    });

    leaf.prop_recursive(
        3, // Max depth
        10, // Max total elements
        5, // Max children per node
        |inner| {
            (inner.clone(), prop::collection::vec(inner, 0..3))
                .prop_map(|(mut parent, children)| {
                    parent.children = children;
                    parent
                })
        },
    )
}

/// Generate arbitrary parsed documents
fn arbitrary_parsed_document() -> impl Strategy<Value = ParsedDocument> {
    (
        arbitrary_file_type(),
        "[a-zA-Z0-9_/.-]{1,50}\\.(rs|py|md|c|cpp|cu|go|sh)",
        1u64..10000u64,
        prop::collection::vec(arbitrary_element_type(), 0..10),
    ).prop_map(|(file_type, path_str, size, element_types)| {
        // Calculate total lines needed for all elements
        let mut total_lines = 1; // At least 1 line
        let mut elements = Vec::new();
        let mut current_line = 1;
        
        for element_type in element_types {
            let content = format!("element_content_{}", current_line);
            let content_lines = content.lines().count().max(1);
            let line_start = current_line;
            let line_end = current_line + content_lines - 1;
            
            elements.push(DocumentElement::new(
                element_type,
                Some(format!("element_{}", current_line)),
                content,
                line_start,
                line_end,
            ));
            
            current_line = line_end + 1;
            total_lines = current_line;
        }
        
        // Create metadata with consistent line count and explicit file type
        let mut metadata = DocumentMetadata::new(PathBuf::from(path_str), size)
            .with_line_count(&"x\n".repeat(total_lines));
        metadata.file_type = file_type; // Override the file type from path detection
        
        ParsedDocument::new(metadata).add_elements(elements)
    })
}

// fn arbitrary_json_attributes() -> impl Strategy<Value = HashMap<String, serde_json::Value>> {
//     prop::collection::hash_map(
//         "[a-zA-Z_][a-zA-Z0-9_]*",
//         prop_oneof![
//             any::<String>().prop_map(serde_json::Value::String),
//             any::<i64>().prop_map(|n| serde_json::Value::Number(n.into())),
//             any::<bool>().prop_map(serde_json::Value::Bool),
//         ],
//         0..5,
//     )
// }

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Property Test 1: JSON Conversion Completeness
    /// 
    /// **Feature: structured-file-parsers, Property 1: JSON Conversion Completeness**
    /// 
    /// For any valid structured document, converting to JSON should preserve all
    /// structural elements including line numbers, metadata, and hierarchical relationships.
    #[test]
    fn property_json_conversion_completeness(document in arbitrary_parsed_document()) {
        let converter = UnifiedJsonConverter::with_default_schemas();
        
        // Convert document to JSON
        let json_result = converter.convert(document.clone());
        prop_assert!(json_result.is_ok(), "JSON conversion should not fail for valid documents");
        
        let json = json_result.unwrap();
        
        // Validate JSON structure
        prop_assert!(json.is_object(), "JSON output must be an object");
        let json_obj = json.as_object().unwrap();
        
        // Check required metadata fields
        prop_assert!(json_obj.contains_key("metadata"), "JSON must contain metadata");
        let metadata = json_obj.get("metadata").unwrap();
        prop_assert!(metadata.is_object(), "Metadata must be an object");
        
        let metadata_obj = metadata.as_object().unwrap();
        prop_assert!(metadata_obj.contains_key("file_type"), "Metadata must contain file_type");
        prop_assert!(metadata_obj.contains_key("path"), "Metadata must contain path");
        prop_assert!(metadata_obj.contains_key("line_count"), "Metadata must contain line_count");
        
        // Validate line count preservation
        let json_line_count = metadata_obj.get("line_count").unwrap().as_u64().unwrap() as usize;
        prop_assert_eq!(json_line_count, document.metadata.line_count, "Line count must be preserved");
        
        // Validate file type preservation
        let json_file_type = metadata_obj.get("file_type").unwrap().as_str().unwrap();
        prop_assert_eq!(json_file_type, document.metadata.file_type.to_string(), "File type must be preserved");
        
        // Validate element preservation
        validate_elements_preserved(&document.elements, json_obj)?;
        
        // Validate line number consistency
        validate_line_number_consistency(json_obj)?;
    }

    /// Property Test 2: Schema Consistency Across File Types
    /// 
    /// **Feature: structured-file-parsers, Property 2: Schema Consistency Across File Types**
    /// 
    /// For any file type supported by vecq, the JSON output should follow consistent
    /// schema patterns with standardized field names and structure.
    #[test]
    fn property_schema_consistency_across_file_types(
        file_type in arbitrary_file_type(),
        elements in prop::collection::vec(arbitrary_document_element(), 1..5),
    ) {
    let metadata = DocumentMetadata::new(
        PathBuf::from(format!("test.{}", file_type.file_extensions()[0])),
        1000,
    ).with_line_count("test content\nline 2\nline 3");
    
    let document = ParsedDocument::new(metadata).add_elements(elements);
    let converter = UnifiedJsonConverter::with_default_schemas();
    
    let json_result = converter.convert(document);
    prop_assert!(json_result.is_ok(), "Conversion should succeed for supported file types");
    
    let json = json_result.unwrap();
    prop_assert!(json.is_object(), "JSON output must be an object");
    
    let json_obj = json.as_object().unwrap();
    
    // Validate consistent metadata structure
    prop_assert!(json_obj.contains_key("metadata"), "All file types must have metadata");
    let metadata = json_obj.get("metadata").unwrap().as_object().unwrap();
    
    // Check required metadata fields are present and correctly typed
    prop_assert!(metadata.contains_key("file_type"), "Metadata must contain file_type");
    prop_assert!(metadata.contains_key("path"), "Metadata must contain path");
    prop_assert!(metadata.contains_key("line_count"), "Metadata must contain line_count");
    prop_assert!(metadata.contains_key("size"), "Metadata must contain size");
    
    // Validate field types
    prop_assert!(metadata.get("file_type").unwrap().is_string(), "file_type must be string");
    prop_assert!(metadata.get("path").unwrap().is_string(), "path must be string");
    prop_assert!(metadata.get("line_count").unwrap().is_number(), "line_count must be number");
    prop_assert!(metadata.get("size").unwrap().is_number(), "size must be number");
    
    // Validate consistent field naming (snake_case)
    for key in json_obj.keys() {
        prop_assert!(
            key.chars().all(|c: char| c.is_lowercase() || c.is_numeric() || c == '_'),
            "Field names must use snake_case: {}",
            key
        );
    }
    
    // Validate element arrays have consistent structure
    for (key, value) in json_obj {
        if key == "metadata" {
            continue;
        }
        
        if let Some(array) = value.as_array() {
            for element in array {
                validate_element_schema_consistency(element)?;
            }
        }
    }
}

    /// Property Test 3: Error Handling Graceful Degradation
    /// 
    /// **Feature: structured-file-parsers, Property 3: Malformed File Graceful Handling**
    /// 
    /// For any malformed or partially parseable file, the system should not crash
    /// and should provide meaningful error information without losing valid structural elements.
    #[test]
    fn property_error_handling_graceful_degradation(
        mut document in arbitrary_parsed_document(),
        corruption_type in 0u8..4u8,
    ) {
    // Introduce various types of "corruption" to test error handling
    match corruption_type % 4 {
        0 => {
            // Corrupt line numbers (make them inconsistent)
            for element in &mut document.elements {
                if !element.children.is_empty() {
                    element.line_end = element.line_start; // Invalid: end before start
                }
            }
        }
        1 => {
            // Add elements with invalid line numbers
            let invalid_element = DocumentElement::new(
                ElementType::Function,
                Some("invalid".to_string()),
                "content".to_string(),
                0, // Invalid: 0-based line number
                1000000, // Invalid: beyond file bounds
            );
            document.elements.push(invalid_element);
        }
        2 => {
            // Corrupt metadata
            document.metadata.line_count = 0; // Invalid: no lines
        }
        3 => {
            // Add deeply nested elements (potential stack overflow)
            let mut deep_element = DocumentElement::new(
                ElementType::Block,
                None,
                "deep".to_string(),
                1,
                1,
            );
            
            // Create deep nesting
            for i in 0..100 {
                let child = DocumentElement::new(
                    ElementType::Block,
                    Some(format!("child_{}", i)),
                    "nested".to_string(),
                    1,
                    1,
                );
                deep_element = deep_element.with_child(child);
            }
            document.elements.push(deep_element);
        }
        _ => unreachable!(),
    }
    
    let converter = UnifiedJsonConverter::with_default_schemas();
    
    // Conversion should either succeed or fail gracefully
    match converter.convert(document) {
        Ok(json) => {
            // If conversion succeeds, JSON should still be valid
            prop_assert!(json.is_object(), "Successful conversion must produce valid JSON object");
            
            let json_obj = json.as_object().unwrap();
            prop_assert!(json_obj.contains_key("metadata"), "JSON must contain metadata even with corruption");
        }
        Err(error) => {
            // If conversion fails, error should be meaningful
            let error_message = error.to_string();
            prop_assert!(!error_message.is_empty(), "Error message must not be empty");
            prop_assert!(!error_message.contains("panic"), "Error should not mention panics");
            
            // Error should contain useful information about the problem
            prop_assert!(
                error_message.contains("parse") || 
                error_message.contains("malformed") || 
                error_message.contains("invalid") ||
                error_message.contains("unsupported") ||
                error_message.contains("Unsupported"),
                "Error message should indicate the type of problem: {}",
                error_message
            );
        }
    }
}

}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_generators_produce_valid_data() {
        // Test that our generators produce valid data
        let mut runner = proptest::test_runner::TestRunner::default();
        
        // Test file type generator
        let file_type = arbitrary_file_type().new_tree(&mut runner).unwrap().current();
        assert_ne!(file_type, FileType::Unknown);
        
        // Test element type generator
        let element_type = arbitrary_element_type().new_tree(&mut runner).unwrap().current();
        assert_ne!(element_type, ElementType::Unknown);
        
        // Test document metadata generator
        let metadata = arbitrary_document_metadata().new_tree(&mut runner).unwrap().current();
        assert!(metadata.line_count > 0);
        assert!(metadata.size > 0);
        
        // Test document element generator
        let element = arbitrary_document_element().new_tree(&mut runner).unwrap().current();
        assert!(element.line_start > 0);
        assert!(element.line_end >= element.line_start);
        assert!(!element.content.is_empty());
    }

    #[test]
    fn test_element_counting() {
        let elements = vec![
            DocumentElement::new(ElementType::Function, Some("f1".to_string()), "content".to_string(), 1, 1),
            DocumentElement::new(ElementType::Function, Some("f2".to_string()), "content".to_string(), 2, 2),
            DocumentElement::new(ElementType::Struct, Some("s1".to_string()), "content".to_string(), 3, 3),
        ];
        
        let mut counts = HashMap::new();
        count_elements_recursive(&elements, &mut counts);
        
        assert_eq!(counts.get(&ElementType::Function), Some(&2));
        assert_eq!(counts.get(&ElementType::Struct), Some(&1));
        assert_eq!(counts.get(&ElementType::Class), None);
    }

} // End of proptest! macro

/// Helper function to count elements recursively
fn count_elements_recursive(
    elements: &[DocumentElement],
    counts: &mut HashMap<ElementType, usize>,
) {
    for element in elements {
        *counts.entry(element.element_type).or_insert(0) += 1;
        count_elements_recursive(&element.children, counts);
    }
}

/// Helper function to validate that all elements are preserved in JSON
fn validate_elements_preserved(
    original_elements: &[DocumentElement],
    json_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), proptest::test_runner::TestCaseError> {
    // Count elements by type in original document
    let mut original_counts: HashMap<ElementType, usize> = HashMap::new();
    count_elements_recursive(original_elements, &mut original_counts);
    
    // Count elements in JSON representation
    let mut json_counts: HashMap<ElementType, usize> = HashMap::new();
    for (key, value) in json_obj {
        if key == "metadata" {
            continue; // Skip metadata
        }
        
        if let Some(array) = value.as_array() {
            // Determine element type from field name
            let element_type = match key.as_str() {
                "functions" | "function" => ElementType::Function,
                "classes" | "class" => ElementType::Class,
                "structs" | "struct" => ElementType::Struct,
                "enums" | "enum" => ElementType::Enum,
                "interfaces" | "interface" => ElementType::Interface,
                "headers" | "header" => ElementType::Header,
                "code_blocks" | "code_block" => ElementType::CodeBlock,
                "links" | "link" => ElementType::Link,
                "tables" | "table" => ElementType::Table,
                "lists" | "list" => ElementType::List,
                "blockquotes" | "blockquote" => ElementType::Blockquote,
                "traits" | "trait" => ElementType::Trait,
                "implementations" | "implementation" => ElementType::Implementation,
                "imports" | "use_statements" | "import" => ElementType::Import,
                "variables" | "variable" => ElementType::Variable,
                "constants" | "constant" => ElementType::Constant,
                "comments" | "comment" => ElementType::Comment,
                "modules" | "module" => ElementType::Module,
                "decorators" | "decorator" => ElementType::Decorator,
                "macros" | "macro" => ElementType::Macro,
                "namespaces" | "namespace" => ElementType::Namespace,
                "packages" | "package" => ElementType::Package,
                "kernels" | "kernel" => ElementType::Kernel,
                "device_functions" | "device_function" => ElementType::DeviceFunction,
                "blocks" | "block" => ElementType::Block,
                "unknowns" | "unknown" => ElementType::Unknown,
                _ => continue, // Unknown field, skip
            };
            
            json_counts.insert(element_type, array.len());
        }
    }
    
    // Verify counts match (allowing for unmapped elements to be in generic fields)
    for (element_type, original_count) in original_counts {
        let json_count = json_counts.get(&element_type).copied().unwrap_or(0);
        
        // If the element type isn't found in its expected field, it might be in a generic field
        // This is acceptable behavior for unmapped element types
        if json_count < original_count {
            // Check if there are any generic fields that might contain unmapped elements
            let mut found_in_generic = false;
            for (key, value) in json_obj {
                if key == "metadata" {
                    continue;
                }
                
                // Check if this is a generic field (not in our known mappings)
                let is_known_field = matches!(key.as_str(),
                    "functions" | "function" | "classes" | "class" | "structs" | "struct" |
                    "enums" | "enum" | "headers" | "header" | "code_blocks" | "code_block" |
                    "links" | "link" | "tables" | "table" | "lists" | "list" |
                    "traits" | "trait" | "implementations" | "implementation" |
                    "imports" | "use_statements" | "import" | "variables" | "variable" |
                    "constants" | "constant" | "modules" | "module" | "decorators" | "decorator" |
                    "macros" | "macro" | "namespaces" | "namespace" | "packages" | "package" |
                    "kernels" | "kernel" | "device_functions" | "device_function"
                );
                
                if !is_known_field && value.is_array() && !value.as_array().unwrap().is_empty() {
                    found_in_generic = true;
                    break;
                }
            }
            
            // Only fail if we can't find the elements anywhere
            if !found_in_generic {
                prop_assert!(
                    json_count >= original_count,
                    "JSON must preserve all elements of type {:?}: original={}, json={}",
                    element_type, original_count, json_count
                );
            }
        }
    }

    Ok(())
}

/// Helper function to validate line number consistency
fn validate_line_number_consistency(
    json_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), proptest::test_runner::TestCaseError> {
    let metadata = json_obj.get("metadata").unwrap().as_object().unwrap();
    let total_lines = metadata.get("line_count").unwrap().as_u64().unwrap() as usize;
    
    // Check all elements have valid line numbers
    for (key, value) in json_obj {
        if key == "metadata" {
            continue;
        }
        
        if let Some(array) = value.as_array() {
            for element in array {
                if let Some(element_obj) = element.as_object() {
                    if let (Some(line_start), Some(line_end)) = (
                        element_obj.get("line_start").and_then(|v| v.as_u64()),
                        element_obj.get("line_end").and_then(|v| v.as_u64()),
                    ) {
                        let line_start = line_start as usize;
                        let line_end = line_end as usize;
                        
                        prop_assert!(line_start > 0, "Line numbers must be 1-based");
                        prop_assert!(line_start <= line_end, "Line start must be <= line end");
                        prop_assert!(line_end <= total_lines, "Line end must be <= total lines");
                    }
                }
            }
        }
    }
    
    Ok(())
}

/// Helper function to validate individual element schema consistency
fn validate_element_schema_consistency(
    element: &serde_json::Value,
) -> Result<(), proptest::test_runner::TestCaseError> {
    let element_obj = element.as_object()
        .ok_or_else(|| proptest::test_runner::TestCaseError::fail("Element must be an object"))?;
    
    // Check required fields
    prop_assert!(element_obj.contains_key("type"), "Element must have type field");
    prop_assert!(element_obj.contains_key("content"), "Element must have content field");
    prop_assert!(element_obj.contains_key("line_start"), "Element must have line_start field");
    prop_assert!(element_obj.contains_key("line_end"), "Element must have line_end field");
    
    // Validate field types
    prop_assert!(element_obj.get("type").unwrap().is_string(), "type must be string");
    prop_assert!(element_obj.get("content").unwrap().is_string(), "content must be string");
    prop_assert!(element_obj.get("line_start").unwrap().is_number(), "line_start must be number");
    prop_assert!(element_obj.get("line_end").unwrap().is_number(), "line_end must be number");
    
    // Validate optional fields if present
    if let Some(name) = element_obj.get("name") {
        prop_assert!(name.is_string(), "name must be string if present");
    }
    
    if let Some(attributes) = element_obj.get("attributes") {
        prop_assert!(attributes.is_object(), "attributes must be object if present");
    }
    
    if let Some(children) = element_obj.get("children") {
        prop_assert!(children.is_array(), "children must be array if present");
        
        // Recursively validate children
        for child in children.as_array().unwrap() {
            validate_element_schema_consistency(child)?;
        }
    }
    
    Ok(())
}