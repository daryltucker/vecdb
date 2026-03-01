use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

// Tier 2: Stank Hunt - Ingest Variants
// Verifies:
// 1. Ingest specific file (not just dir)
// 2. Custom collection flag
// 3. Metadata flags (Stank Hunt fix)

#[test]
fn test_ingest_variants() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let file_path = root.join("specific.rs");
    fs::write(&file_path, "fn specific() {}").unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));

    cmd.arg("ingest")
        .arg(file_path.to_str().unwrap())
        .arg("--collection")
        .arg("test_ingest_variants_col")
        .arg("-m")
        .arg("author=daryl")
        .arg("-m")
        .arg("status=active")
        .arg("--dry-run");

    let assert = cmd.assert();

    assert
        .success()
        .stdout(predicate::str::contains("specific.rs"))
        // Check for metadata in output (implemented in previous step)
        .stdout(predicate::str::contains("author").and(predicate::str::contains("daryl")))
        .stdout(predicate::str::contains("status").and(predicate::str::contains("active")));
}
