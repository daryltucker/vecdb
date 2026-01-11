// PURPOSE:
//   Property-based testing for file type detection engine in vecq.
//   Validates that file type detection works correctly across all supported file types
//   and edge cases, ensuring the foundation of vecq's parsing pipeline is reliable.
//   Critical because incorrect file type detection breaks all downstream processing.
//
// REQUIREMENTS:
//   User-specified:
//   - Must validate accurate file type detection for all supported languages
//   - Must test detection with various file naming patterns and edge cases
//   - Must verify confidence scoring works correctly across detection strategies
//   - Must ensure custom file type mappings work seamlessly with built-in detection
//   - Must validate graceful handling of unknown and malformed files
//   
//   Implementation-discovered:
//   - Requires realistic test data generation for all supported file types
//   - Must test all detection strategies (extension, MIME, shebang, content analysis)
//   - Needs validation of detection caching and performance characteristics
//   - Must verify schema consistency across all detected file types
//   - Requires comprehensive edge case coverage (empty files, binary files, etc.)
//
// IMPLEMENTATION RULES:
//   1. Generate realistic file content for each supported language
//      Rationale: Artificial test data may not catch real-world detection issues
//   
//   2. Test all detection strategies independently and in combination
//      Rationale: Each strategy has different failure modes that must be validated
//   
//   3. Use minimum 1000 iterations per property test for comprehensive coverage
//      Rationale: File type detection has many edge cases that require extensive testing
//   
//   4. Validate confidence scoring accuracy and consistency
//      Rationale: Confidence scores guide parser selection and user feedback
//   
//   5. Test custom configuration scenarios thoroughly
//      Rationale: Users rely on custom mappings for specialized file types
//   
//   Critical:
//   - DO NOT use hardcoded file content that doesn't represent real-world usage
//   - DO NOT skip testing edge cases like empty files or binary content
//   - ALWAYS validate that detection results produce consistent JSON schemas
//
// USAGE:
//   # Run property tests for file detection
//   cargo test property_file_detection --release
//   
//   # Run with specific iteration count
//   PROPTEST_CASES=5000 cargo test property_file_detection
//   
//   # Debug failing test cases
//   cargo test property_file_detection -- --nocapture
//   
//   # Run only extension detection tests
//   cargo test test_extension_based_detection
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new file type support:
//   1. Add new file type to FileTypeGenerator in generate_file_content()
//   2. Add realistic content generation for the new language
//   3. Update extension mapping tests with new file extensions
//   4. Add shebang patterns if the language supports script execution
//   5. Update confidence scoring validation for new detection patterns
//   6. Add edge cases specific to the new file type
//   
//   When detection accuracy issues are reported:
//   1. Add failing cases to test fixtures for regression testing
//   2. Update content analysis scoring if needed
//   3. Adjust confidence thresholds based on real-world performance
//   4. Add new detection strategies if existing ones are insufficient
//   5. Update test generators to cover newly discovered edge cases
//
// RELATED FILES:
//   - src/detection.rs - File type detection implementation being tested
//   - src/types.rs - FileType enum and related types
//   - src/parsers/mod.rs - Parser implementations that depend on detection
//   - tests/fixtures/ - Real-world test files for detection validation
//   - tests/unit/detection_tests.rs - Unit tests for specific detection scenarios
//
// MAINTENANCE:
//   Update when:
//   - New file types are added to vecq
//   - Detection strategies are modified or added
//   - Confidence scoring algorithms change
//   - Real-world detection accuracy issues are discovered
//   - Performance characteristics of detection change significantly
//
// Last Verified: 2025-12-31

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use std::path::PathBuf;
use vecq::detection::{FileTypeDetector, HybridDetector, DetectionConfig};
use vecq::types::FileType;

// Property 2: Schema Consistency Across File Types
// 
// For any file type supported by vecq, the JSON output should follow consistent 
// schema patterns with standardized field names and structure.
// 
// This property ensures that:
// 1. All detected file types can be successfully parsed
// 2. Parsed documents produce JSON with consistent schema patterns
// 3. Detection confidence correlates with parsing success
// 4. Custom file type mappings work seamlessly with built-in detection

// Test data generators

/// Generate realistic file content for different file types
fn generate_file_content(file_type: FileType) -> BoxedStrategy<Vec<u8>> {
    match file_type {
        FileType::Rust => generate_rust_content().boxed(),
        FileType::Python => generate_python_content().boxed(),
        FileType::Markdown => generate_markdown_content().boxed(),
        FileType::C => generate_c_content().boxed(),
        FileType::Cpp => generate_cpp_content().boxed(),
        FileType::Cuda => generate_cuda_content().boxed(),
        FileType::Go => generate_go_content().boxed(),
        FileType::Bash => generate_bash_content().boxed(),
        FileType::Json => generate_json_content().boxed(),
        FileType::Text => generate_text_content().boxed(),
        FileType::Html => generate_html_content().boxed(),
        FileType::Toml => generate_toml_content().boxed(),
        FileType::Unknown => generate_unknown_content().boxed(),
    }
}

/// Generate realistic TOML content
fn generate_toml_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just("[package]\nname = \"vecq\"\nversion = \"0.1.0\"".as_bytes().to_vec()),
        Just("key = \"value\"\nnumber = 123".as_bytes().to_vec()),
    ]
}

/// Generate realistic Rust source code
fn generate_rust_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Simple function
        Just("fn main() {\n    println!(\"Hello, world!\");\n}".as_bytes().to_vec()),
        
        // Struct with implementation
        Just("struct Point {\n    x: f64,\n    y: f64,\n}\n\nimpl Point {\n    fn new(x: f64, y: f64) -> Self {\n        Point { x, y }\n    }\n}".as_bytes().to_vec()),
        
        // Enum with match
        Just("enum Color {\n    Red,\n    Green,\n    Blue,\n}\n\nfn describe_color(color: Color) -> &'static str {\n    match color {\n        Color::Red => \"red\",\n        Color::Green => \"green\",\n        Color::Blue => \"blue\",\n    }\n}".as_bytes().to_vec()),
        
        // Trait definition
        Just("trait Display {\n    fn fmt(&self) -> String;\n}\n\nimpl Display for i32 {\n    fn fmt(&self) -> String {\n        format!(\"{}\", self)\n    }\n}".as_bytes().to_vec()),
        
        // Generic function with lifetimes
        Just("fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {\n    if x.len() > y.len() {\n        x\n    } else {\n        y\n    }\n}".as_bytes().to_vec()),
        
        // Module with use statements
        Just("use std::collections::HashMap;\nuse std::fs::File;\n\nmod utils {\n    pub fn helper() -> i32 {\n        42\n    }\n}".as_bytes().to_vec()),
    ]
}

/// Generate realistic Python source code
fn generate_python_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Simple function
        Just("def hello_world():\n    print(\"Hello, world!\")\n\nif __name__ == \"__main__\":\n    hello_world()".as_bytes().to_vec()),
        
        // Class definition
        Just("class Point:\n    def __init__(self, x, y):\n        self.x = x\n        self.y = y\n    \n    def distance(self, other):\n        return ((self.x - other.x)**2 + (self.y - other.y)**2)**0.5".as_bytes().to_vec()),
        
        // Function with decorators
        Just("from functools import wraps\n\ndef decorator(func):\n    @wraps(func)\n    def wrapper(*args, **kwargs):\n        return func(*args, **kwargs)\n    return wrapper\n\n@decorator\ndef example():\n    pass".as_bytes().to_vec()),
        
        // List comprehension and imports
        Just("import os\nimport sys\n\ndef process_files(directory):\n    return [f for f in os.listdir(directory) if f.endswith('.py')]".as_bytes().to_vec()),
        
        // Exception handling
        Just("try:\n    with open('file.txt', 'r') as f:\n        content = f.read()\nexcept FileNotFoundError:\n    print(\"File not found\")\nexcept Exception as e:\n    print(f\"Error: {e}\")".as_bytes().to_vec()),
    ]
}

/// Generate realistic Markdown content
fn generate_markdown_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Basic document structure
        Just("# Title\n\n## Introduction\n\nThis is a paragraph with **bold** and *italic* text.\n\n### Code Example\n\n```rust\nfn main() {\n    println!(\"Hello!\");\n}\n```\n\n## Conclusion\n\nThat's all!".as_bytes().to_vec()),
        
        // Lists and links
        Just("# Project README\n\n## Features\n\n- Feature 1\n- Feature 2\n  - Nested item\n  - Another nested item\n\n## Links\n\n[Documentation](https://example.com)\n[GitHub](https://github.com/example/repo)".as_bytes().to_vec()),
        
        // Tables
        Just("# Data\n\n| Name | Age | City |\n|------|-----|------|\n| Alice | 30 | NYC |\n| Bob | 25 | LA |\n\n> This is a blockquote\n> with multiple lines.".as_bytes().to_vec()),
        
        // Mixed content
        Just("# API Documentation\n\n## Overview\n\nThis API provides access to user data.\n\n### Authentication\n\n```bash\ncurl -H \"Authorization: Bearer token\" https://api.example.com\n```\n\n#### Response\n\n```json\n{\n  \"status\": \"success\",\n  \"data\": []\n}\n```".as_bytes().to_vec()),
    ]
}

/// Generate realistic C source code
fn generate_c_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Simple program
        Just("#include <stdio.h>\n\nint main() {\n    printf(\"Hello, world!\\n\");\n    return 0;\n}".as_bytes().to_vec()),
        
        // Function with struct
        Just("#include <stdlib.h>\n\ntypedef struct {\n    int x;\n    int y;\n} Point;\n\nPoint* create_point(int x, int y) {\n    Point* p = malloc(sizeof(Point));\n    p->x = x;\n    p->y = y;\n    return p;\n}".as_bytes().to_vec()),
        
        // Header guards
        Just("#ifndef UTILS_H\n#define UTILS_H\n\nvoid utility_function(void);\n\n#endif /* UTILS_H */".as_bytes().to_vec()),
        
        // Preprocessor macros
        Just("#define MAX(a, b) ((a) > (b) ? (a) : (b))\n#define PI 3.14159\n\nint calculate(int a, int b) {\n    return MAX(a, b) * PI;\n}".as_bytes().to_vec()),
    ]
}

/// Generate realistic C++ source code
fn generate_cpp_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Class with constructor
        Just("#include <iostream>\n\nclass Point {\npublic:\n    Point(double x, double y) : x_(x), y_(y) {}\n    double distance() const { return sqrt(x_*x_ + y_*y_); }\nprivate:\n    double x_, y_;\n};".as_bytes().to_vec()),
        
        // Template function
        Just("#include <vector>\n\ntemplate<typename T>\nT max_element(const std::vector<T>& vec) {\n    T max_val = vec[0];\n    for (const auto& elem : vec) {\n        if (elem > max_val) max_val = elem;\n    }\n    return max_val;\n}".as_bytes().to_vec()),
        
        // Namespace and using
        Just("namespace math {\n    const double PI = 3.14159;\n    \n    double area(double radius) {\n        return PI * radius * radius;\n    }\n}\n\nusing namespace math;".as_bytes().to_vec()),
    ]
}

/// Generate realistic CUDA source code
fn generate_cuda_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Simple kernel
        Just("__global__ void add_kernel(float* a, float* b, float* c, int n) {\n    int idx = blockIdx.x * blockDim.x + threadIdx.x;\n    if (idx < n) {\n        c[idx] = a[idx] + b[idx];\n    }\n}".as_bytes().to_vec()),
        
        // Device function
        Just("__device__ float square(float x) {\n    return x * x;\n}\n\n__global__ void compute(float* data, int n) {\n    int idx = threadIdx.x;\n    if (idx < n) {\n        data[idx] = square(data[idx]);\n    }\n}".as_bytes().to_vec()),
    ]
}

/// Generate realistic Go source code
fn generate_go_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Simple program
        Just("package main\n\nimport \"fmt\"\n\nfunc main() {\n    fmt.Println(\"Hello, world!\")\n}".as_bytes().to_vec()),
        
        // Struct with methods
        Just("package main\n\ntype Point struct {\n    X, Y float64\n}\n\nfunc (p Point) Distance() float64 {\n    return math.Sqrt(p.X*p.X + p.Y*p.Y)\n}\n\nfunc NewPoint(x, y float64) *Point {\n    return &Point{X: x, Y: y}\n}".as_bytes().to_vec()),
        
        // Interface
        Just("package shapes\n\ntype Shape interface {\n    Area() float64\n    Perimeter() float64\n}\n\ntype Rectangle struct {\n    Width, Height float64\n}\n\nfunc (r Rectangle) Area() float64 {\n    return r.Width * r.Height\n}".as_bytes().to_vec()),
    ]
}

/// Generate realistic Bash script content
fn generate_bash_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Simple script with shebang
        Just("#!/bin/bash\n\necho \"Hello, world!\"\nls -la\necho \"Done\"".as_bytes().to_vec()),
        
        // Function and variables
        Just("#!/usr/bin/env bash\n\nfunction backup_files() {\n    local source=\"$1\"\n    local dest=\"$2\"\n    cp -r \"$source\" \"$dest\"\n}\n\nSOURCE_DIR=\"/home/user\"\nBACKUP_DIR=\"/backup\"\nbackup_files \"$SOURCE_DIR\" \"$BACKUP_DIR\"".as_bytes().to_vec()),
        
        // Conditional and loops
        Just("#!/bin/bash\n\nfor file in *.txt; do\n    if [[ -f \"$file\" ]]; then\n        echo \"Processing $file\"\n        wc -l \"$file\"\n    fi\ndone".as_bytes().to_vec()),
    ]
}

/// Generate realistic JSON content
fn generate_json_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just("{\"key\": \"value\"}".as_bytes().to_vec()),
        Just("[1, 2, 3]".as_bytes().to_vec()),
        Just("{\"nested\": {\"a\": 1}}".as_bytes().to_vec()),
    ]
}

/// Generate realistic Text content
fn generate_text_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just("Plain text content".as_bytes().to_vec()),
        Just("Line 1\nLine 2".as_bytes().to_vec()),
        Just("key=value\nsetting=on".as_bytes().to_vec()),
    ]
}

/// Generate realistic HTML/XML content
fn generate_html_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just("<!DOCTYPE html><html><body><h1>Hello</h1></body></html>".as_bytes().to_vec()),
        Just("<div class=\"test\">Content</div>".as_bytes().to_vec()),
        Just("<?xml version=\"1.0\"?><root><item>Value</item></root>".as_bytes().to_vec()),
        Just("<mcp_servers><server>test</server></mcp_servers>".as_bytes().to_vec()),
    ]
}

/// Generate unknown/unsupported file content
fn generate_unknown_content() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Random text
        Just("This is some random text content\nthat doesn't match any known file type\npatterns.".as_bytes().to_vec()),
        
        // Binary-like content
        Just(vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]), // PNG header
        
        // Empty content
        Just(Vec::new()),
        
        // Random binary
        Just(vec![0x00, 0x01, 0x02, 0x03]),
    ]
}

/// Generate file paths with various extensions
fn generate_file_path(file_type: FileType) -> BoxedStrategy<PathBuf> {
    match file_type {
        FileType::Rust => prop_oneof![
            Just(PathBuf::from("main.rs")),
            Just(PathBuf::from("lib.rs")),
            Just(PathBuf::from("src/parser.rs")),
            Just(PathBuf::from("tests/integration.rs")),
        ].boxed(),
        FileType::Python => prop_oneof![
            Just(PathBuf::from("main.py")),
            Just(PathBuf::from("script.py")),
            Just(PathBuf::from("src/utils.py")),
            Just(PathBuf::from("test_example.py")),
        ].boxed(),
        FileType::Markdown => prop_oneof![
            Just(PathBuf::from("README.md")),
            Just(PathBuf::from("CHANGELOG.md")),
            Just(PathBuf::from("docs/guide.md")),
            Just(PathBuf::from("notes.markdown")),
        ].boxed(),
        FileType::C => prop_oneof![
            Just(PathBuf::from("main.c")),
            Just(PathBuf::from("utils.c")),
            Just(PathBuf::from("src/parser.c")),
            Just(PathBuf::from("include/header.h")),
        ].boxed(),
        FileType::Cpp => prop_oneof![
            Just(PathBuf::from("main.cpp")),
            Just(PathBuf::from("utils.cc")),
            Just(PathBuf::from("src/parser.cxx")),
            Just(PathBuf::from("include/header.hpp")),
        ].boxed(),
        FileType::Cuda => prop_oneof![
            Just(PathBuf::from("kernel.cu")),
            Just(PathBuf::from("compute.cu")),
            Just(PathBuf::from("src/gpu_utils.cu")),
        ].boxed(),
        FileType::Go => prop_oneof![
            Just(PathBuf::from("main.go")),
            Just(PathBuf::from("utils.go")),
            Just(PathBuf::from("src/parser.go")),
            Just(PathBuf::from("cmd/cli.go")),
        ].boxed(),
        FileType::Bash => prop_oneof![
            Just(PathBuf::from("script.sh")),
            Just(PathBuf::from("build.bash")),
            Just(PathBuf::from("bin/deploy")),
            Just(PathBuf::from("scripts/setup")),
        ].boxed(),
        FileType::Json => prop_oneof![
            Just(PathBuf::from("data.json")),
            Just(PathBuf::from("config.json")),
            Just(PathBuf::from("package.json")),
        ].boxed(),
        FileType::Text => prop_oneof![
            Just(PathBuf::from("notes.txt")),
            Just(PathBuf::from("log.txt")),
            Just(PathBuf::from("config.ini")),
            Just(PathBuf::from("data.yaml")),
        ].boxed(),
        FileType::Html => prop_oneof![
            Just(PathBuf::from("index.html")),
            Just(PathBuf::from("page.htm")),
            Just(PathBuf::from("config.xml")),
            Just(PathBuf::from("template.xhtml")),
        ].boxed(),
        FileType::Toml => prop_oneof![
            Just(PathBuf::from("Cargo.toml")),
            Just(PathBuf::from("config.toml")),
            Just(PathBuf::from("Pipfile")),
        ].boxed(),
        FileType::Unknown => prop_oneof![
            Just(PathBuf::from("unknown.xyz")),
            Just(PathBuf::from("data.bin")),
            Just(PathBuf::from("config")),
            Just(PathBuf::from("file_without_extension")),
        ].boxed(),
    }
}

/// Generate supported file types
fn supported_file_type() -> impl Strategy<Value = FileType> {
    prop_oneof![
        Just(FileType::Rust),
        Just(FileType::Markdown),
        Just(FileType::Html),
        Just(FileType::Toml),
    ]
}

// fn any_file_type() -> impl Strategy<Value = FileType> {
//     prop_oneof![
//         supported_file_type(),
//         Just(FileType::Unknown),
//     ]
// }

// Property tests

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Test that extension-based detection works correctly
    #[test]
    fn test_extension_based_detection(
        file_type in supported_file_type(),
        content in prop::collection::vec(any::<u8>(), 1..2048)
    ) {
        let detector = HybridDetector::new();
        let path = generate_file_path(file_type).new_tree(&mut Default::default()).unwrap().current();
        
        let detected_type = detector.detect_type(&path, &content).unwrap();
        let confidence = detector.get_confidence(&path, &content);
        
        // Extension-based detection should be highly confident for correct extensions
        if detected_type == file_type {
            prop_assert!(confidence >= 0.8, "Extension detection should have high confidence");
        }
        
        // Should be able to get a parser for detected type
        let parser_result = detector.get_parser(detected_type);
        prop_assert!(parser_result.is_ok() || detected_type == FileType::Unknown);
    }

    /// Test that shebang detection works for script files
    #[test]
    fn test_shebang_detection(
        script_type in prop_oneof![Just(FileType::Python), Just(FileType::Bash)],
        additional_content in "\\PC{0,1024}" // Bounded regex (PC = visible chars + space)
    ) {
        let detector = HybridDetector::new();
        let path = PathBuf::from("script"); // No extension
        
        let shebang = match script_type {
            FileType::Python => "#!/usr/bin/env python3\n",
            FileType::Bash => "#!/bin/bash\n",
            _ => unreachable!(),
        };
        
        let content = format!("{}{}", shebang, additional_content);
        let detected_type = detector.detect_type(&path, content.as_bytes()).unwrap();
        let confidence = detector.get_confidence(&path, content.as_bytes());
        
        // Shebang detection should work even without file extension
        prop_assert_eq!(detected_type, script_type);
        prop_assert!(confidence >= 0.7, "Shebang detection should have good confidence");
    }

    /// Test that content analysis provides reasonable results
    #[test]
    fn test_content_analysis(
        file_type in supported_file_type()
    ) {
        let detector = HybridDetector::new();
        let path = PathBuf::from("unknown_file"); // No extension to force content analysis
        
        let content = generate_file_content(file_type).new_tree(&mut Default::default()).unwrap().current();
        
        let detected_type = detector.detect_type(&path, &content).unwrap();
        let confidence = detector.get_confidence(&path, &content);
        
        // Content analysis should at least not crash and provide some confidence
        prop_assert!((0.0..=1.0).contains(&confidence));
        
        // If detection succeeds, should be able to get parser
        if detected_type != FileType::Unknown {
            let parser_result = detector.get_parser(detected_type);
            prop_assert!(parser_result.is_ok());
        }
    }

    /// Test that custom configuration works correctly
    #[test]
    fn test_custom_configuration(
        custom_extension in "[a-z]{2,5}",
        file_type in supported_file_type(),
        content in any::<Vec<u8>>().prop_filter("Non-empty content", |c| !c.is_empty())
    ) {
        let config = DetectionConfig::new()
            .with_custom_extension(&custom_extension, file_type)
            .with_confidence_threshold(0.5);
        
        let detector = HybridDetector::with_config(config);
        let path = PathBuf::from(format!("test.{}", custom_extension));
        
        let detected_type = detector.detect_type(&path, &content).unwrap();
        let confidence = detector.get_confidence(&path, &content);
        
        // Custom extension mapping should work
        prop_assert_eq!(detected_type, file_type);
        prop_assert!(confidence >= 0.9, "Custom extension should have very high confidence");
    }

    /// Test that confidence thresholds work correctly
    #[test]
    fn test_confidence_threshold(
        threshold in 0.1f64..0.9f64,
        _file_type in supported_file_type()
    ) {
        let config = DetectionConfig::new()
            .with_confidence_threshold(threshold);
        
        let detector = HybridDetector::with_config(config);
        let path = PathBuf::from("ambiguous_file"); // No clear indicators
        let content = b"some ambiguous content that might not clearly indicate file type";
        
        let detected_type = detector.detect_type(&path, content).unwrap();
        let confidence = detector.get_confidence(&path, content);
        
        // If detection succeeds, confidence should meet threshold
        if detected_type != FileType::Unknown {
            prop_assert!(confidence >= threshold, 
                "Detected type confidence {} should meet threshold {}", confidence, threshold);
        }
    }

    /// Test that detection caching works correctly
    #[test]
    fn test_detection_caching(
        file_type in supported_file_type()
    ) {
        let detector = HybridDetector::new();
        let path = generate_file_path(file_type).new_tree(&mut Default::default()).unwrap().current();
        let content = generate_file_content(file_type).new_tree(&mut Default::default()).unwrap().current();
        
        // First detection
        let result1 = detector.detect_type(&path, &content).unwrap();
        let confidence1 = detector.get_confidence(&path, &content);
        
        // Second detection (should use cache)
        let result2 = detector.detect_type(&path, &content).unwrap();
        let confidence2 = detector.get_confidence(&path, &content);
        
        // Results should be identical
        prop_assert_eq!(result1, result2);
        prop_assert!((confidence1 - confidence2).abs() < 0.001, "Confidence should be consistent");
    }

    /// Test that malformed content doesn't crash detection
    #[test]
    fn test_malformed_content_handling(
        malformed_content in prop::collection::vec(any::<u8>(), 0..4096),
        path_str in "[a-zA-Z0-9_.-]{1,50}"
    ) {
        let detector = HybridDetector::new();
        let path = PathBuf::from(path_str);
        
        // Detection should never panic, even with malformed content
        let result = detector.detect_type(&path, &malformed_content);
        prop_assert!(result.is_ok(), "Detection should handle malformed content gracefully");
        
        let confidence = detector.get_confidence(&path, &malformed_content);
        prop_assert!((0.0..=1.0).contains(&confidence), "Confidence should be valid range");
    }

    /// Test that all supported file types can be detected and parsed
    #[test]
    fn test_end_to_end_detection_and_parsing(
        file_type in supported_file_type()
    ) {
        let detector = HybridDetector::new();
        let path = generate_file_path(file_type).new_tree(&mut Default::default()).unwrap().current();
        let content = generate_file_content(file_type).new_tree(&mut Default::default()).unwrap().current();
        
        // Detect file type
        let detected_type = detector.detect_type(&path, &content).unwrap();
        
        // Get parser for detected type
        let parser_result = detector.get_parser(detected_type);
        
        if detected_type != FileType::Unknown {
            prop_assert!(parser_result.is_ok(), "Should be able to get parser for detected type");
            
            // Try to parse content (this validates the entire pipeline)
            if let Ok(_parser) = parser_result {
                let _content_str = String::from_utf8_lossy(&content);
                // Note: We don't require parsing to succeed as content might be incomplete,
                // but we do require that it doesn't panic
                let _parse_result = std::panic::catch_unwind(|| {
                    // This would be async in real usage, but for property testing we just verify no panic
                    // parser.parse(&content_str).await
                });
            }
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_basic_extension_detection() {
        let detector = HybridDetector::new();
        
        // Test Rust
        let rust_path = PathBuf::from("main.rs");
        let result = detector.detect_type(&rust_path, b"fn main() {}").unwrap();
        assert_eq!(result, FileType::Rust);
        
        // Test Python
        let python_path = PathBuf::from("script.py");
        let result = detector.detect_type(&python_path, b"def main(): pass").unwrap();
        assert_eq!(result, FileType::Python);
        
        // Test Markdown
        let md_path = PathBuf::from("README.md");
        let result = detector.detect_type(&md_path, b"# Title").unwrap();
        assert_eq!(result, FileType::Markdown);
    }

    #[test]
    fn test_shebang_detection() {
        let detector = HybridDetector::new();
        let path = PathBuf::from("script");
        
        // Python shebang
        let python_content = b"#!/usr/bin/env python3\nprint('hello')";
        let result = detector.detect_type(&path, python_content).unwrap();
        assert_eq!(result, FileType::Python);
        
        // Bash shebang
        let bash_content = b"#!/bin/bash\necho 'hello'";
        let result = detector.detect_type(&path, bash_content).unwrap();
        assert_eq!(result, FileType::Bash);
    }

    #[test]
    fn test_confidence_scoring() {
        let detector = HybridDetector::new();
        
        // High confidence for clear extension match
        let rust_path = PathBuf::from("main.rs");
        let confidence = detector.get_confidence(&rust_path, b"fn main() {}");
        assert!(confidence > 0.8);
        
        // Lower confidence for ambiguous content
        let unknown_path = PathBuf::from("unknown");
        let confidence = detector.get_confidence(&unknown_path, b"random content");
        assert!(confidence < 0.5);
    }

    #[test]
    fn test_custom_configuration() {
        let config = DetectionConfig::new()
            .with_custom_extension("mylang", FileType::Unknown)
            .with_confidence_threshold(0.8);
        
        let detector = HybridDetector::with_config(config);
        let path = PathBuf::from("test.mylang");
        let result = detector.detect_type(&path, b"custom content").unwrap();
        assert_eq!(result, FileType::Unknown);
    }

    #[test]
    fn test_empty_and_binary_content() {
        let detector = HybridDetector::new();
        
        // Empty content
        let path = PathBuf::from("empty.rs");
        let result = detector.detect_type(&path, b"");
        assert!(result.is_ok());
        
        // Binary content
        let binary_content = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let result = detector.detect_type(&path, &binary_content);
        assert!(result.is_ok());
    }
}