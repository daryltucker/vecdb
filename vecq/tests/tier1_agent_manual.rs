//! The agent manual documents the contract, and there is exactly one of it.
//!
//! `vecq man --agent` embeds `vecq/src/docs/man_agent.md`; the published
//! `docs/vecq/AGENT_MANUAL.md` must be the same text. They were two files and
//! they drifted: each had sections the other lacked, the public one told
//! agents `.links[]` was `{ text, url }` and to query `.paragraph[]`, and both
//! led with `select(.visibility == "pub")` — which returns nothing, because
//! visibility is `attributes.visibility`. One text, checked here.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn manual_from_binary() -> String {
    let out = Command::new(common::vecq_bin())
        .args(["man", "--agent"])
        .output()
        .expect("vecq runs");
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn the_published_manual_is_the_one_the_binary_prints() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let embedded = std::fs::read_to_string(root.join("src/docs/man_agent.md")).unwrap();
    let published = std::fs::read_to_string(root.join("../docs/vecq/AGENT_MANUAL.md")).unwrap();
    assert!(
        embedded == published,
        "docs/vecq/AGENT_MANUAL.md differs from vecq/src/docs/man_agent.md — edit the latter \
         and copy it over; they are one document"
    );
    assert_eq!(manual_from_binary().trim_end(), embedded.trim_end());
}

/// V7: the contract consumers rely on is in the manual, as sections — not as
/// passing mentions.
#[test]
fn v7_the_manual_documents_the_contract() {
    let manual = manual_from_binary();
    let hits = manual
        .lines()
        .filter(|l| {
            [
                "heading_path",
                "syntax_errors",
                "--source-info",
                "\"ERROR\"",
                "refs",
            ]
            .iter()
            .any(|k| l.contains(k))
        })
        .count();
    assert!(hits >= 5, "V7: {hits} lines");
    for section in [
        "## THE ELEMENT CONTRACT",
        "### 3. Calls, strings and references",
        "**1-indexed and inclusive**",
        "`metadata.syntax_errors`",
        "**`--source-info`**",
        "**`vecq refs <SYMBOL>",
        "**`.module`**",
        "`.attributes.heading_path`",
        "--no-content",
    ] {
        assert!(manual.contains(section), "manual lacks: {section}");
    }
    assert!(
        !manual.contains("select(.visibility =="),
        "top-level `.visibility` does not exist; it is `.attributes.visibility`"
    );
    assert!(
        !manual.contains(".paragraph[]"),
        "the key is `.paragraphs[]`"
    );
}

/// Every language the manual says has calls[] does, and every type vecq
/// detects is one the manual can be checked against (`vecq elements`).
#[test]
fn the_manual_names_every_language_with_calls() {
    let manual = manual_from_binary();
    for lang in ["Rust", "Python", "Go", "C++", "CUDA", "Bash", "JavaScript"] {
        assert!(manual.contains(lang), "{lang}");
    }
    assert!(manual.contains("not TypeScript"));
}
