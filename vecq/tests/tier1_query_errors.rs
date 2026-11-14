//! A query that fails is a failure, never an empty answer.
//!
//! Until 2026-314 the query engine ran `filter_map(|r| r.ok())` over jaq's
//! results: every runtime error was dropped. `.name | .x`, `.functions[]` on a
//! Markdown file, a stdlib program tripping over a new key — all printed
//! nothing and exited 0, exactly what "no matches" looks like. That hid a real
//! regression the same day: `vecq doc` went blank when the converter gained
//! the `module` object, and nothing said why. jq itself reports the error and
//! exits non-zero; so does vecq now, per input, carrying on with the rest.

mod common;

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn vecq(args: &[&str]) -> Output {
    Command::new(common::vecq_bin())
        .args(args)
        .current_dir(fixtures())
        .output()
        .expect("vecq runs")
}

#[test]
fn a_runtime_error_is_reported_and_exits_non_zero() {
    let out = vecq(&["refs/b.rs", "-q", ".functions[0].name | .x"]);
    assert!(!out.status.success(), "a failing query must not exit 0");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("jq error"), "{stderr}");
    assert!(
        stderr.contains(".functions[0].name | .x"),
        "the error names the query: {stderr}"
    );
    assert!(out.stdout.is_empty());
}

/// Over a mixed tree, the inputs a query does not fit are named on stderr, the
/// others still produce output, and the run exits non-zero — jq's behaviour.
/// `?` (or `// []`) is how a query says "absent is fine", and then it is.
#[test]
fn a_mixed_tree_reports_each_failing_file_and_keeps_going() {
    let out = vecq(&["-R", "source_contract", "-q", ".functions[] | .name"]);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("\"alpha\""),
        "the Rust/Python files still answer: {stdout}"
    );
    assert!(
        stderr.contains("probe.md"),
        "the Markdown file is named: {stderr}"
    );

    let tolerant = vecq(&["-R", "source_contract", "-q", ".functions[]? | .name"]);
    assert!(
        tolerant.status.success(),
        "{}",
        String::from_utf8_lossy(&tolerant.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&tolerant.stdout), stdout);
}

/// The regression the swallowed errors hid: `vecq doc` must render a file
/// whose JSON has the `module` scope object beside its element arrays.
#[test]
fn doc_renders_despite_the_module_scope_object() {
    let out = vecq(&["doc", "refs/b.rs"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let page = String::from_utf8_lossy(&out.stdout);
    assert!(
        page.contains("### beta") && page.contains("pub fn beta() -> u32"),
        "{page}"
    );
}

/// `-R` does not walk into hidden directories (`.git`, `.vecdb`, `.venv`), as
/// `vecq map` and `vecq refs` already do not; a hidden file named on the
/// command line is still read. Found by the error reporting above: `-R` over
/// a fixture tree had been parsing ingest state under `.vecdb/` all along.
#[test]
fn recursion_skips_hidden_entries_but_not_named_paths() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".hidden")).unwrap();
    std::fs::write(dir.path().join(".hidden/inner.rs"), "fn hidden() {}\n").unwrap();
    std::fs::write(dir.path().join("shown.rs"), "fn shown() {}\n").unwrap();
    let names = |out: Output| {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let root = dir.path().to_str().unwrap();
    assert_eq!(
        names(vecq(&["-R", root, "-q", ".functions[].name"])),
        "\"shown\""
    );
    let hidden = dir.path().join(".hidden/inner.rs");
    assert_eq!(
        names(vecq(&[hidden.to_str().unwrap(), "-q", ".functions[].name"])),
        "\"hidden\""
    );
}

/// A reader that stops early (`vecq … | head`) is success, not an error: the
/// final flush reported "Error: Broken pipe" and exited 1 (101 — a panic —
/// when stderr was closed too), while every earlier write already treated a
/// closed pipe as exit 0.
#[test]
fn a_reader_that_stops_early_is_success() {
    use std::process::Stdio;
    // Deterministic: output under the 8 KB stdout buffer, so its only real
    // write IS the final flush, and the reader is gone before it happens.
    // (Megabytes of output hit the per-line path first — which always
    // handled a closed pipe — and made the test pass on the broken binary.)
    let mut child = Command::new(common::vecq_bin())
        .args(["refs/b.rs", "-q", ".functions[].name"])
        .current_dir(fixtures())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "exit {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}
