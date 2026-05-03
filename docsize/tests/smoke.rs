// Smoke tests for the docsize binary.
// These verify the binary compiles and basic CLI flags work without requiring
// a running Ollama/vecdb instance. They are intentionally minimal — docsize is
// an optional example utility, not a core workspace member.

use std::process::Command;

fn docsize_bin() -> Command {
    let bin = env!("CARGO_BIN_EXE_docsize");
    Command::new(bin)
}

#[test]
fn help_flag_exits_cleanly() {
    let output = docsize_bin()
        .arg("--help")
        .output()
        .expect("failed to run docsize --help");
    assert!(
        output.status.success(),
        "docsize --help exited with non-zero status: {:?}",
        output.status
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("docsize"),
        "expected 'docsize' in --help output, got: {stdout}"
    );
}

#[test]
fn version_flag_exits_cleanly() {
    let output = docsize_bin()
        .arg("--version")
        .output()
        .expect("failed to run docsize --version");
    assert!(
        output.status.success(),
        "docsize --version exited with non-zero status: {:?}",
        output.status
    );
    // clap emits "docsize X.Y.Z" — verify the binary name and Cargo version appear
    let stdout = String::from_utf8_lossy(&output.stdout);
    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        stdout.contains(expected_version),
        "expected version '{expected_version}' in --version output, got: {stdout}"
    );
}
