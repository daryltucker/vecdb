use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

// Tier 2: Stank Hunt - Ignore Vectorignore Flag
// Verifies that `--ignore-vectorignore` causes .vectorignore to be skipped
// during file walking, so files that would normally be excluded are ingested.

#[test]
fn test_ignore_vectorignore_flag() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create files:
    // - keep.rs
    // - secret.rs  (listed in .vectorignore)
    // - target/debug/lib.rs  (within dir listed in .vectorignore)

    fs::write(root.join("keep.rs"), "fn main() {}").unwrap();
    fs::write(root.join("secret.rs"), "secret data").unwrap();

    let target = root.join("target");
    let debug = target.join("debug");
    fs::create_dir_all(&debug).unwrap();
    fs::write(debug.join("lib.rs"), "fn lib() {}").unwrap();

    // Create .vectorignore to exclude "secret.rs" and "target/"
    fs::write(root.join(".vectorignore"), "secret.rs\ntarget/").unwrap();

    // WITHOUT --ignore-vectorignore: secret.rs and target/ should be excluded
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.arg("ingest")
        .arg(root.to_str().unwrap())
        .arg("-c")
        .arg("test_vecignore_respected")
        .arg("--dry-run");

    let assert = cmd.assert();
    assert
        .success()
        // keep.rs should appear
        .stdout(predicate::str::contains("keep.rs"))
        // secret.rs should be excluded via .vectorignore
        .stdout(predicate::str::contains("secret.rs").not())
        // target/debug/lib.rs should be excluded via .vectorignore
        .stdout(predicate::str::contains("target/debug/lib.rs").not());

    // WITH --ignore-vectorignore: secret.rs and target/ should NOT be excluded
    let mut cmd2 = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd2.arg("ingest")
        .arg(root.to_str().unwrap())
        .arg("-c")
        .arg("test_vecignore_ignored")
        .arg("--dry-run")
        .arg("--ignore-vectorignore");

    let assert2 = cmd2.assert();
    assert2
        .success()
        // keep.rs should still appear
        .stdout(predicate::str::contains("keep.rs"))
        // secret.rs should now appear (not excluded)
        .stdout(predicate::str::contains("secret.rs"))
        // target/debug/lib.rs should now appear (not excluded)
        .stdout(predicate::str::contains("target/debug/lib.rs"));
}
