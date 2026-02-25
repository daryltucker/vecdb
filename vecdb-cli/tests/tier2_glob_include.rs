use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

// Tier 2: Stank Hunt - Globbing Includes
// Verifies that 'ingest' respects explicit file extension filters.

#[test]
fn test_ingest_extensions_include() {
    // 1. Setup fixture
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create files:
    // - included.rs
    // - ignored.txt
    // - deeper/included.rs
    // - deeper/ignored.md

    fs::write(root.join("included.rs"), "fn main() {}").unwrap();
    fs::write(root.join("ignored.txt"), "text file").unwrap();

    let deep = root.join("deeper");
    fs::create_dir(&deep).unwrap();
    fs::write(deep.join("included.rs"), "fn deep() {}").unwrap();
    fs::write(deep.join("ignored.md"), "# markdown").unwrap();

    // 2. Run CLI with --extensions rs
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.arg("ingest")
        .arg(root.to_str().unwrap())
        .arg("--extensions")
        .arg("rs") // Only ingest .rs files
        .arg("--collection")
        .arg("test_glob_include")
        // Use a dry-run if available, or force a failure on connection but check output?
        // Currently no dry-run, so we expect it to try to connect.
        // However, we can check the OUTPUT for "Ingesting content from: ...".
        // Ideally we want to see "Processing included.rs" vs "Skipping ignored.txt".
        // But standard output might not be granular enough without verbose?
        // Let's assume we implement a --dry-run flag that prints files found.
        .arg("--dry-run");

    // Expectation:
    // If --dry-run is implemented, it should succeed execution.
    // Parsing should succeed.
    // Output should contain "included.rs" and NOT "ignored.txt".

    let assert = cmd.assert();

    assert
        .success()
        // Non-interactive output is just the path
        .stdout(predicate::str::contains("included.rs"))
        .stdout(predicate::str::contains("[Dry Run]").not())
        .stdout(predicate::str::contains("ignored.txt").not())
        .stdout(predicate::str::contains("deeper/included.rs"))
        .stdout(predicate::str::contains("deeper/ignored.md").not());
}
