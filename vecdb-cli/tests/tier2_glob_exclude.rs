use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

// Tier 2: Stank Hunt - Globbing Excludes
// Verifies that 'ingest' respects explicit excludes and .vectorignore.

#[test]
fn test_ingest_excludes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create files:
    // - keep.rs
    // - ignore_me.rs
    // - target/file.rs
    // - .vectorignore

    fs::write(root.join("keep.rs"), "fn main() {}").unwrap();
    fs::write(root.join("ignore_me.rs"), "ignore me").unwrap();

    let target = root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("file.rs"), "fn target() {}").unwrap();

    // Create .vectorignore to exclude "target"
    fs::write(root.join(".vectorignore"), "target/").unwrap();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.arg("ingest")
        .arg(root.to_str().unwrap())
        .arg("--excludes")
        .arg("ignore_me.rs") // Explicit exclude via flag
        .arg("--dry-run"); // Use dry-run to verify

    let assert = cmd.assert();

    assert
        .success()
        // Should keep keep.rs
        .stdout(predicate::str::contains("keep.rs"))
        // Should exclude ignore_me.rs (flag)
        .stdout(predicate::str::contains("ignore_me.rs").not())
        // Should exclude target/file.rs (.vectorignore)
        .stdout(predicate::str::contains("target/file.rs").not());
}
