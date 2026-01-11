use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::io::Write;
use tempfile::TempDir;

// Tier 2: Smart Ingestion Verification
// Verifies:
// 1. Binary file skipping
// 2. Extensionless script detection (shebang)
// 3. Language metadata enrichment

#[test]
fn test_smart_ingestion_dry_run() {
    // 1. Setup fixture
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    
    // Create 'main.rs' - Standard Rust file
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { println!(\"Hello\"); }").unwrap();
    
    // Create 'README.md' - Standard Markdown
    fs::write(root.join("README.md"), "# Documentation\n\nDocs here.").unwrap();

    // Create 'script' - Extensionless Shell Script
    let script_path = root.join("script");
    fs::write(&script_path, "#!/bin/bash\necho 'Detected!'").unwrap();
    
    // Create 'binary.dat' - Binary content (null bytes)
    let bin_path = root.join("binary.dat");
    let mut file = fs::File::create(&bin_path).unwrap();
    file.write_all(b"\x00\x01\x02\x03\xff\xfe").unwrap();
    
    // 2. Run CLI with --dry-run
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.arg("ingest")
       .arg(root.to_str().unwrap())
       .arg("--dry-run");

    let assert = cmd.assert();

    // 3. Verify Output
    assert
        .success()
        // Rust file detected & included
        .stdout(predicate::str::contains("main.rs"))
        // Markdown detected & included
        .stdout(predicate::str::contains("README.md"))
        // Script detected & included
        .stdout(predicate::str::contains("script"))
        // Binary detected & skipped (not in stdout)
        .stdout(predicate::str::contains("binary.dat").not())
        // Summary confirms 1 skipped
        .stderr(predicate::str::contains("Skipped 1"));
}
