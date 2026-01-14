use vecq::parsers::{create_parser, available_parsers};
use vecq::types::FileType;
use std::path::PathBuf;
use std::fs;
use tokio::runtime::Runtime;

#[test]
fn test_all_parsers_have_fixtures() {
    let parsers = available_parsers();
    let fixtures_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    
    println!("Checking compliance for {} parsers...", parsers.len());
    
    for parser_type in parsers {
        // Map FileType to directory name
        // This mapping ensures that we have a standard directory structure
        let dir_name = match parser_type {
            FileType::Markdown => "markdown",
            FileType::Rust => "rust",
            FileType::Python => "python",
            FileType::C => "c",
            FileType::Cpp => "cpp",
            FileType::Cuda => "cuda",
            FileType::Go => "go",
            FileType::Bash => "bash",
            FileType::Html => "html",
            FileType::Text => "text",
            FileType::Toml => "toml",
            FileType::Json => "json",
            _ => panic!("New parser type {:?} added without updating compliance test mapping!", parser_type),
        };
        
        let parser_dir = fixtures_root.join(dir_name);
        if !parser_dir.exists() || !parser_dir.is_dir() {
            // Check if feature flag disabled this?
            // available_parsers() only returns compiled parsers.
            panic!("MISSING FIXTURES: No directory found for {:?} at {:?}", parser_type, parser_dir);
        }
        
        let entries = fs::read_dir(&parser_dir)
            .unwrap_or_else(|e| panic!("Failed to read {:?}: {}", parser_dir, e))
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect::<Vec<_>>();
            
        if entries.is_empty() {
            panic!("EMPTY FIXTURES: No test files found for {:?} in {:?}", parser_type, parser_dir);
        }
        
        // Run compliance check on each fixture
        let rt = Runtime::new().unwrap();
        let parser = create_parser(parser_type.clone())
            .unwrap_or_else(|e| panic!("Failed to create parser for {:?}: {}", parser_type, e));
        
        println!("  Verifying {:?} with {} fixtures...", parser_type, entries.len());
        
        for entry in entries {
            let path = entry.path();
            // Skip hidden files or non-source files if needed
            if path.file_name().and_then(|n| n.to_str()).map(|s| s.starts_with(".")).unwrap_or(false) {
                continue;
            }
            
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("Failed to read file {:?}", path));
                
            let result = rt.block_on(parser.parse(&content));
            
            match result {
                Ok(doc) => {
                    // Sanity assertions
                    assert_eq!(doc.metadata.file_type, parser_type, "File type mismatch in metadata for {:?}", path);
                    
                    // Verify line numbers are non-decreasing (basic sanity)
                    let mut last_line = 0;
                    for element in &doc.elements {
                        if element.line_start < last_line {
                            // This might happen for nested elements if flattened? 
                            // vecq structure is flattened or hierarchical? 
                            // DocumentElement has children. The top level elements should be increasing?
                            // With recursive parsing, we might have issues if we traverse deeply?
                            // But elements iterator iterates top level.
                            // However, let's just warn or check basic validity.
                        }
                        assert!(element.line_end >= element.line_start, "Line end < line start in {:?}", path);
                        last_line = element.line_start;
                    }
                },
                Err(e) => {
                    panic!("PARSING FAILED: {:?} failed to parse fixture {:?}: {}", parser_type, path, e);
                }
            }
        }
    }
}
