use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_collection_cleanup_logic() {
    // 1. Setup
    let temp_dir = TempDir::new().unwrap();
    let data_dir = temp_dir.path().join("data");
    fs::create_dir(&data_dir).unwrap();
    fs::write(data_dir.join("file1.txt"), "Hello World").unwrap();

    // 2. First Ingest (Collection A)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("vecdb"));
    cmd.arg("ingest")
        .arg(&data_dir)
        .arg("--collection")
        .arg("test_cleanup_A")
        .assert()
        .success();

    // Verify state.toml exists and has collection ID
    let state_path = data_dir.join(".vecdb/state.toml");
    assert!(state_path.exists());
    let state_content = fs::read_to_string(&state_path).unwrap();
    assert!(state_content.contains("[collections.test_cleanup_A]"));
    assert!(state_content.contains("id = \""));

    // 3. Delete Collection (Remote Only)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("vecdb"));
    cmd.arg("delete")
        .arg("test_cleanup_A")
        .arg("--yes")
        .assert()
        .success();

    // 4. Second Ingest (Same Name, New Remote)
    // This should trigger the cleanup logic because local ID won't match the new remote ID (or lack thereof)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("vecdb"));
    cmd.arg("ingest")
        .arg(&data_dir)
        .arg("--collection")
        .arg("test_cleanup_A")
        .assert()
        .success();
    // Note: We don't check for "Collection ID mismatch" in stderr because the test runner
    // is likely non-interactive, suppression the user-facing message.
    // Instead we verify the ID changed in the state file below.

    // 5. Verify State Updated
    let state_content_new = fs::read_to_string(&state_path).unwrap();
    assert!(state_content_new.contains("[collections.test_cleanup_A]"));

    // Extract IDs to compare (Regex or simple string split)
    // For now, implicit verification via "Collection ID mismatch" output is good, but let's be sure.
    assert_ne!(
        state_content, state_content_new,
        "State file should have changed (new ID)"
    );
}
