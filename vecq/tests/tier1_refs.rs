//! Calls, strings and references.
//!
//! Before this an agent mapping a codebase — every SQL call, every caller of
//! `alpha` — had vecq up to the function and a regex after it: `content` was a
//! string, so "where is this called" was a grep, and a grep matches comments
//! and string literals that only mention the name. These run the RFCs' own
//! acceptance commands against the built binary, over fixtures checked in
//! under `tests/fixtures/refs/`:
//!
//! - `probe.py` is the probe, verbatim.
//! - `a.rs` / `b.rs` are V6's two-file fixture: `b.rs` calls `alpha()` — as a
//!   plain call, by full path, as a method, and inside a macro's arguments —
//!   and mentions it in a comment and a string, which must NOT count.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn refs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/refs")
}

fn vecq(args: &[&str]) -> String {
    let out = Command::new(common::vecq_bin())
        .args(args)
        .current_dir(refs_dir())
        .output()
        .expect("vecq runs");
    assert!(
        out.status.success(),
        "vecq {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn json(args: &[&str]) -> serde_json::Value {
    let out = vecq(args);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?}: not JSON ({e}): {out}"))
}

/// First command: every `execute` call's first argument —
/// literal, f-string with its hole as `{}`, a name, and a `+` fold.
///
/// `unique`: a method is listed both in `.functions[]` and under its class, so
/// `..` meets it twice (the RFC's expected list is sorted and distinct).
/// `"UPDATE jobs SET x = 1"`: the RFC wrote `"UPDATE {}jobs SET x = 1"`, but
/// the probe's `"UPDATE " + "jobs SET x = 1"` is two literals and no hole —
/// a `{}` there would be a hole that does not exist.
#[test]
fn rfc_294_every_execute_call_with_its_sql() {
    let got = json(&[
        "probe.py",
        "-q",
        r#"[.. | .calls? // empty | .[] | select(.callee|endswith("execute")) | .args[0].value] | unique"#,
    ]);
    assert_eq!(
        got,
        serde_json::json!([
            "DELETE FROM job_queue WHERE id = ?",
            "INSERT INTO {} (a) VALUES (?)",
            "PAGE_SQL",
            "UPDATE jobs SET x = 1"
        ])
    );
    let insert = json(&[
        "probe.py",
        "-q",
        r#"[.. | .calls? // empty | .[] | select(.args[0].value? == "INSERT INTO {} (a) VALUES (?)") | .args[0]][0]"#,
    ]);
    assert_eq!(insert["holes"], serde_json::json!(["table"]));
    let update = json(&[
        "probe.py",
        "-q",
        r#"[.. | .strings? // empty | .[] | select(.value == "UPDATE jobs SET x = 1")][0]"#,
    ]);
    assert_eq!(update["folded"], true, "{update}");
}

/// Second command, and §4.3: the module-level call is the
/// module's, named `<module>` — not dropped, not attributed to a function.
#[test]
fn rfc_294_refs_finds_the_module_level_connect() {
    let out = vecq(&["refs", "sqlite3.connect", "."]);
    assert_eq!(out.trim(), "./probe.py:4 <module>  call sqlite3.connect");
    let module = json(&["probe.py", "-q", ".module"]);
    assert_eq!(module["name"], "<module>");
    let callees: Vec<&str> = module["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["callee"].as_str().unwrap())
        .collect();
    assert_eq!(callees, ["sqlite3.connect", "conn.execute"]);
}

/// The module statement itself is an element of its own (`.blocks[]`), as
/// written — defect 2 stays fixed without the old `<module>`
/// element that spanned other elements.
#[test]
fn module_statements_are_blocks_with_their_kind() {
    let blocks = json(&[
        "probe.py",
        "-q",
        "[.blocks[] | {kind: .attributes.kind, content}]",
    ]);
    assert_eq!(
        blocks,
        serde_json::json!([{
            "kind": "expression_statement",
            "content": "conn.execute(\"DELETE FROM job_queue WHERE id = ?\", (1,))"
        }])
    );
}

/// `vecq refs alpha` over the two-file fixture lists every
/// call site with file and span — and nothing that merely mentions the name.
#[test]
fn v6_refs_lists_every_call_site_with_file_and_span() {
    let report = json(&["refs", "alpha", "a.rs", "b.rs", "--json"]);
    let refs = report["references"].as_array().unwrap();
    let got: Vec<(String, String, u64, Vec<String>)> = refs
        .iter()
        .map(|r| {
            (
                r["kind"].as_str().unwrap().to_string(),
                r["name"].as_str().unwrap().to_string(),
                r["line"].as_u64().unwrap(),
                r["scope"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect();
    let row = |k: &str, n: &str, l: u64, s: &[&str]| {
        (
            k.to_string(),
            n.to_string(),
            l,
            s.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
        )
    };
    assert_eq!(
        got,
        vec![
            row("import", "crate::a::alpha", 1, &["<module>"]),
            row("call", "alpha", 6, &["beta"]),
            row("call", "crate::a::alpha", 6, &["beta"]),
            // Inside `assert_eq!(..)`'s token tree — not lost to the macro.
            row("call", "alpha", 11, &["impl Thing", "m"]),
            row("call", "self.alpha", 12, &["impl Thing", "m"]),
        ],
        "comment, string literal and `alphabet()` must not match: {report:#}"
    );
    // Every reference carries the file and an exact span of the call.
    let source = std::fs::read_to_string(refs_dir().join("b.rs")).unwrap();
    for r in refs {
        assert_eq!(r["file"], "b.rs");
        let (s, e) = (
            r["start"].as_u64().unwrap() as usize,
            r["end"].as_u64().unwrap() as usize,
        );
        let text = &source[s..e];
        assert!(
            text.contains(r["name"].as_str().unwrap().trim_end_matches('!'))
                || r["kind"] == "import",
            "span {s}..{e} = {text:?} is not the reference {r}"
        );
    }
    assert_eq!(report["failed"], serde_json::json!([]));
}

/// Second half: `.functions[0].calls` is non-empty for a
/// function that calls another; and `calls` is present (empty) on one that
/// calls nothing, so `.functions[].calls[]` never trips on a missing key.
#[test]
fn v6_a_function_lists_its_calls() {
    let calls = json(&["b.rs", "-q", ".functions[0].calls | map(.callee)"]);
    assert_eq!(
        calls,
        serde_json::json!(["alpha", "crate::a::alpha", "alphabet"])
    );
    let none = json(&["a.rs", "-q", "[.functions[].calls | length]"]);
    assert_eq!(none, serde_json::json!([0, 0]));
}

/// The library API behind the CLI (R6: "not CLI-only") — what ivaldi calls.
#[tokio::test]
async fn library_find_references_matches_the_cli() {
    let report = vecq::find_references(&[refs_dir().join("b.rs")], "alpha")
        .await
        .unwrap();
    assert_eq!(report.files_searched, 1);
    assert_eq!(report.references.len(), 5);
    assert!(report.failed.is_empty());
    let doc = vecq::parse_file(
        &std::fs::read_to_string(refs_dir().join("probe.py")).unwrap(),
        vecq::FileType::Python,
    )
    .await
    .unwrap();
    let imports: Vec<_> = vecq::references_in(&doc, "sqlite3")
        .into_iter()
        .filter(|r| r.kind == vecq::ReferenceKind::Import)
        .collect();
    assert_eq!(
        imports.len(),
        1,
        "`import sqlite3` is a reference to sqlite3"
    );
}

/// A file that fails to parse is reported, not silently skipped: a refs answer
/// that quietly missed a file is a wrong answer.
#[test]
fn a_file_that_does_not_parse_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.py"), "def (:\n").unwrap();
    std::fs::write(dir.path().join("ok.py"), "alpha()\n").unwrap();
    let out = Command::new(common::vecq_bin())
        .args(["refs", "alpha", "--json"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["files_searched"], 2);
    assert_eq!(report["references"].as_array().unwrap().len(), 1);
    assert_eq!(report["failed"].as_array().unwrap().len(), 1, "{report:#}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad.py"));
}

/// JavaScript (registered 2026-314): ES imports and `new` are references too —
/// "who imports better-sqlite3, and who constructs Database?"
#[test]
fn javascript_imports_and_constructors_are_references() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/javascript/module.js");
    let report = json(&["refs", "Database", fixture.to_str().unwrap(), "--json"]);
    let got: Vec<(String, u64, String)> = report["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["kind"].as_str().unwrap().to_string(),
                r["line"].as_u64().unwrap(),
                r["scope"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("."),
            )
        })
        .collect();
    assert_eq!(
        got,
        [("call".to_string(), 26, "Store.constructor".to_string())]
    );
    let imports = json(&[
        "refs",
        "better-sqlite3",
        fixture.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(imports["references"][0]["kind"], "import");
}

/// REGRESSION: one call of `alpha` written as a chain was
/// reported once per link — `alpha`, `alpha(1).map`,
/// `alpha(1).map(|v| v + 1).unwrap_or` — because the call's parentheses were
/// stripped before matching and `alpha(1)` then looked like a module prefix.
/// `alpha(1).map` calls `map` on alpha's RESULT; it is not a call site of
/// `alpha`. ivaldi measured 25 references for 13 real call sites.
#[test]
fn a_method_chain_is_one_call_site_of_its_head() {
    let out = vecq(&["refs", "alpha", "chain.rs"]);
    assert_eq!(out.trim(), "chain.rs:4 beta  call alpha");
}
