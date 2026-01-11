use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;
use std::path::Path;

// INTEGRATION TEST: vecdb-cli
// Implements the "Tier 3 Fresh Install Journey" in Rust.
// Verifies Init -> Ingest (Docs + External) -> Search -> Verify.

#[test]
fn test_cli_help() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Vector Database Project CLI"));
}

#[test]
fn test_cli_version() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.arg("--version")
        .assert()
        .success();
}

#[test]
fn test_fresh_install_journey() {
    // 1. SETUP ISOLATED ENVIRONMENT
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path().join("config");
    let data_dir = temp_dir.path().join("data");
    
    // Override XDG paths to enforce isolation
    // CRITICAL: Use the Test Qdrant Port (6335) to avoid breaking production
    let envs = vec![
        ("XDG_CONFIG_HOME", config_dir.to_str().unwrap()),
        ("XDG_DATA_HOME", data_dir.to_str().unwrap()),
        ("QDRANT_URL", "http://localhost:6336"), 
        ("VECDB_INTERACTIVE", "0"), // Disable TTY features
        ("RUST_BACKTRACE", "1"),
    ];

    // 2. INIT
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.envs(envs.clone())
        .arg("init")
        .assert()
        .success();

    // Verify config file created
    let config_file = config_dir.join("vecdb/config.toml");
    assert!(config_file.exists(), "vecdb init failed to create config file");

    // 3. CREATE CONTENT (Small Doc)
    let docs_dir = temp_dir.path().join("docs");
    fs::create_dir(&docs_dir).unwrap();
    let file_path = docs_dir.join("hello.md");
    fs::write(&file_path, "# Hello World\nThis is a test document about bananas.").unwrap();

    let collection = "test_tier3_fresh_install";
    
    // Clean up first (Safety)
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.envs(envs.clone())
        .arg("delete")
        .arg(collection)
        .arg("--yes")
        .assert()
        .success();

    // 4. INGEST (Small Doc)
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.envs(envs.clone())
        .arg("ingest")
        .arg(&docs_dir)
        .arg("--collection")
        .arg(collection)
        .assert()
        .success()
        // We match "Ingestion Summary" because that's what the harness fixed
        .stderr(predicate::str::contains("Ingestion Summary"));

    // 5. SEARCH (Small Doc)
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    let assert = cmd.envs(envs.clone())
        .arg("search")
        .arg("bananas")
        .arg("--collection")
        .arg(collection)
        .arg("--json")
        .assert()
        .success();
    
    let output = assert.get_output();
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    assert!(stdout.contains("bananas"), "Search output missing keyword");


    // 6. HIGH FIDELITY VERIFICATION (External Fixtures)
    // Attempt to locate ../tests/fixtures/external
    // We are in vecdb-cli/ (package root) or workspace root. 
    // Let's try finding the workspace root relative to CARGO_MANIFEST_DIR
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let package_root = Path::new(&manifest_dir);
    // Workspace root is likely one level up
    let workspace_root = package_root.parent().unwrap();
    let external_fixtures = workspace_root.join("tests/fixtures/external");

    if !external_fixtures.exists() {
        println!("SKIPPING: External fixtures not found at {:?}", external_fixtures);
        return;
    }

    // 6a. Lua Ingestion (Code)
    let lua_path = external_fixtures.join("lua-5.4.6");
    if lua_path.exists() {
        println!("--- Ingesting Lua Source ---");
        // Always clean local state for re-ingestion test
        let local_state = lua_path.join(".vecdb");
        if local_state.exists() {
            let _ = fs::remove_dir_all(&local_state);
        }

        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
        cmd.envs(envs.clone())
            .arg("ingest")
            .arg(&lua_path)
            .arg("--collection")
            .arg(collection)
            .assert()
            .success();

        // Search for lua_newstate
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
        let assert = cmd.envs(envs.clone())
            .arg("search")
            .arg("lua_newstate")
            .arg("--collection")
            .arg(collection)
            .arg("--json")
            .assert()
            .success();
        let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
        assert!(out.contains("lua_newstate"), "Failed to find lua_newstate");
    }

    // 6b. Pride and Prejudice (Text)
    let pandp_path = external_fixtures.join("pride-and-prejudice.txt");
    if pandp_path.exists() {
         println!("--- Ingesting Pride and Prejudice ---");
         let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
         cmd.envs(envs.clone())
            .arg("ingest")
            .arg(&pandp_path)
            .arg("--collection")
            .arg(collection)
            .assert()
            .success();

         // Search for Elizabeth
         let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
         let assert = cmd.envs(envs.clone())
            .arg("search")
            .arg("Elizabeth")
            .arg("--collection")
            .arg(collection)
            .arg("--json")
            .assert()
            .success();
         let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
         assert!(out.contains("Elizabeth"), "Failed to find Elizabeth");
    }

    // 7. CLEANUP
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecdb"));
    cmd.envs(envs.clone())
        .arg("delete")
        .arg(collection)
        .arg("--yes")
        .assert()
        .success();
}
