//! Workspace layering: who is allowed to understand a file format.
//!
//! ## The rule
//!
//! **Format and language support has exactly one implementation, and it is
//! vecq's.** `vecdb-core` may depend on `vecq` and does; what it must not do is
//! carry a competing parser for a file type vecq already handles.
//!
//! ## Why this replaced `test_no_vecq_dependency_in_core`
//!
//! This file used to assert the opposite edge:
//!
//! ```ignore
//! panic!("ARCHITECTURE VIOLATION: vecdb-core MUST NOT depend on vecq!");
//! ```
//!
//! The goal behind it was right — vecq is meant to be installable on its own and
//! stay lightweight, and Ivaldi consumes it directly for AST-aware editing. But
//! forbidding the *dependency* rather than the *duplication* pushed things the
//! wrong way, twice:
//!
//! 1. The vecq→chunk adapter could not live in `vecdb-core`, the crate both
//!    binaries share. So it was copied into `vecdb-cli` and `vecdb-server`, and
//!    the copies drifted — the server's keyed chunk IDs on `line_start`, so
//!    inserting a line at the top of a file re-identified every chunk below it
//!    and an MCP re-ingest duplicated the whole file. Ivaldi drives the MCP
//!    server.
//!
//! 2. `vecdb-core` grew its own `json.rs` and `yaml.rs` behind a
//!    `BuiltinParserFactory` that no binary ever constructed, while vecq had its
//!    own JSON and TOML parsers. Two implementations with different support
//!    levels — core's handled comments and trailing commas, vecq's did not — and
//!    the core test suite pointed at the unreachable one. It reported that
//!    JSON-with-comments worked while every real `tsconfig.json` silently
//!    degraded to a single unstructured text chunk.
//!
//! Both failures are duplication, not dependency. So duplication is what is
//! pinned here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn core_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The dependency edge the retired rule forbade must actually exist.
///
/// Without it the adapter cannot live in `vecdb-core`, and the last time it
/// could not, it ended up copied into both binaries and the copies forked.
#[test]
fn core_depends_on_vecq() {
    let manifest = core_root().join("Cargo.toml");
    let content = std::fs::read_to_string(&manifest).expect("failed to read vecdb-core/Cargo.toml");
    let cargo: toml::Value =
        toml::from_str(&content).expect("failed to parse vecdb-core/Cargo.toml");

    let deps = cargo
        .get("dependencies")
        .and_then(|d| d.as_table())
        .expect("vecdb-core must declare dependencies");

    assert!(
        deps.contains_key("vecq"),
        "vecdb-core no longer depends on vecq. If that is intentional, the vecq \
         adapter has to live somewhere else — and the last time it did, it was \
         copied into both binaries and the copies drifted into keying chunk IDs \
         differently. Read this file's module docs before removing this."
    );
}

/// `vecdb-core` must not reimplement a format vecq already parses.
///
/// Checked structurally: a file under `src/parsers/` named after a format is a
/// parser for that format. `streaming_json` is the one allowed exception and
/// says why in its own entry.
#[test]
fn core_ships_no_competing_format_parsers() {
    // Formats vecq owns. Names match vecq/src/parsers/*.rs.
    const VECQ_OWNS: &[&str] = &[
        "json",
        "toml",
        "yaml",
        "markdown",
        "html",
        "text",
        "rust",
        "python",
        "go",
        "c",
        "cpp",
        "cuda",
        "bash",
        "javascript",
    ];

    // The single permitted exception, with the reason it is not a second opinion.
    let allowed: BTreeMap<&str, &str> = [(
        "streaming_json",
        "not a second opinion about JSON structure — the only way to read a file \
         too large to hold in memory as an AST. Reachable via get_streaming_parser.",
    )]
    .into_iter()
    .collect();

    let parsers_dir = core_root().join("src/parsers");
    let mut violations = Vec::new();

    for entry in std::fs::read_dir(&parsers_dir)
        .expect("vecdb-core/src/parsers must exist")
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        if stem == "mod" || stem == "vecq_adapter" || allowed.contains_key(stem.as_str()) {
            continue;
        }

        if VECQ_OWNS.contains(&stem.as_str()) {
            violations.push(stem);
        }
    }

    assert!(
        violations.is_empty(),
        "vecdb-core/src/parsers contains parsers for formats vecq already owns: {violations:?}\n\n\
         Two parsers for one format means two support levels, and the one that \
         ships is whichever the factory happens to reach. That is how \
         vecdb-core's JSON parser came to handle comments that vecq's rejected, \
         while vecq's was the only one any binary used — the tests passed against \
         a parser that never ran.\n\n\
         Add the capability to vecq instead. If a file here genuinely is not a \
         competing implementation, add it to `allowed` with the reason."
    );
}

/// Every binary must inject the same factory.
///
/// A second factory is how a second parser becomes reachable, which is the step
/// that turns dead duplicate code into divergent behaviour.
#[test]
fn binaries_inject_only_the_vecq_factory() {
    let workspace = core_root().parent().expect("workspace root").to_path_buf();
    let mut offenders = Vec::new();

    for crate_name in ["vecdb-cli", "vecdb-server"] {
        let src = workspace.join(crate_name).join("src");
        collect_factory_uses(&src, &mut offenders);
    }

    assert!(
        offenders.is_empty(),
        "these files construct a parser factory other than VecqParserFactory:\n{offenders:#?}\n\n\
         There must be one, so that what a binary understands about a file type \
         does not depend on which binary it is."
    );
}

fn collect_factory_uses(dir: &Path, offenders: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_factory_uses(&path, offenders);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("//") {
                continue;
            }
            if line.contains("ParserFactory") && !line.contains("VecqParserFactory") {
                offenders.push(format!("{}:{}: {}", path.display(), n + 1, line));
            }
        }
    }
}
