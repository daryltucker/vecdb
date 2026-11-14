//! `vecq map` graphs every declaration kind the parsers emit.
//!
//! The JGF normalizer (`src/stdlib/normalizers/graph_src.jq`) reads the parsed
//! document by key. A key it reads that no parser emits is a silent hole: the
//! kind parses, `vecq` lists it, and the project graph never shows it. Rust
//! `impl` blocks were that hole — the normalizer read `.impls` while the
//! document key is `implementations`. And `map -o json`, documented as the
//! full graph, printed the pruned architectural one, so the hole could not be
//! seen from the CLI at all. This test holds both: the normalizer to the
//! document keys, and the two output modes to their documented contents.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn node_types(args: &[&str]) -> Vec<String> {
    let out = Command::new(common::vecq_bin())
        .args(args)
        .output()
        .expect("vecq runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("output is JSON");
    let nodes = v["graphs"][0]["nodes"]
        .as_object()
        .expect("JGF v2: nodes is an object keyed by id");
    let mut types: Vec<String> = nodes
        .values()
        .filter_map(|n| n["metadata"]["type"].as_str().map(str::to_string))
        .collect();
    types.sort();
    types.dedup();
    types
}

fn rust_fixtures() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/rust")
        .to_str()
        .unwrap()
        .to_string()
}

/// `src_to_graph` on a file that declares an `impl` yields an implementation node.
#[test]
fn the_normalizer_graphs_rust_impl_blocks() {
    let file =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust/declaration_kinds.rs");
    let types = node_types(&[file.to_str().unwrap(), "-q", "src_to_graph"]);
    assert!(
        types.iter().any(|t| t == "implementation"),
        "src_to_graph has no implementation node for a file with an impl; seen {types:?}"
    );
}

/// `map -o json` is the full graph: every declaration kind the fixtures declare.
#[test]
fn map_json_is_the_full_symbol_graph() {
    let dir = rust_fixtures();
    let types = node_types(&["map", &dir, "-o", "json"]);
    for kind in [
        "function",
        "structure",
        "module",
        "implementation",
        "enumeration",
        "trait",
    ] {
        assert!(
            types.contains(&kind.to_string()),
            "map -o json lacks {kind}; seen {types:?}"
        );
    }
}

/// `map -o graph` is the pruned architectural graph: structure only, no symbols.
#[test]
fn map_graph_is_pruned_to_architecture() {
    let dir = rust_fixtures();
    let types = node_types(&["map", &dir, "-o", "graph"]);
    for kind in ["file", "module", "structure"] {
        assert!(
            types.contains(&kind.to_string()),
            "map -o graph lacks {kind}; seen {types:?}"
        );
    }
    for kind in ["function", "implementation"] {
        assert!(
            !types.contains(&kind.to_string()),
            "map -o graph is pruned; it must not carry {kind}; seen {types:?}"
        );
    }
}
