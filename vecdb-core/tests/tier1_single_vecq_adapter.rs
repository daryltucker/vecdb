//! There must be exactly one bridge from vecq's AST to vecdb chunks.
//!
//! There were two. `vecdb-cli/src/vecq_adapter.rs` and
//! `vecdb-server/src/vecq_adapter.rs` began as a copy and drifted:
//!
//! | | CLI | server |
//! |---|---|---|
//! | chunk ID seed | `doc_id::crumbtrail::content_hash` | `doc_id::line_start::crumbtrail` |
//! | parent-redundancy filter | present | absent |
//! | `get_streaming_parser` | present | absent |
//!
//! Keying a chunk ID on `line_start` means inserting a line at the top of a file
//! re-identifies every chunk below it. Nothing deletes the points the old IDs
//! occupied, so an MCP re-ingest after any edit duplicated the whole file.
//! Ivaldi drives the MCP server, so Ivaldi got the worst of all three — and
//! nothing failed, because no test compared the two files or read a collection
//! back after an edit.
//!
//! Duplication is the root cause, so duplication is what this test forbids.
//! Assertions about *behaviour* would have to be written twice to catch a fork,
//! which is the same mistake one level up.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is vecdb-core/; the workspace is its parent.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("vecdb-core must live inside the workspace")
        .to_path_buf()
}

/// Walk the workspace's crate sources looking for anything that re-implements
/// the adapter, wherever it is put next time.
fn find_adapter_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();

        if path.is_dir() {
            // `target/` is build output, not a second implementation.
            if name == "target" || name == ".git" {
                continue;
            }
            find_adapter_sources(&path, found);
        } else if name == "vecq_adapter.rs" {
            found.push(path);
        }
    }
}

#[test]
fn exactly_one_vecq_adapter_exists() {
    let root = workspace_root();
    let mut found = Vec::new();
    find_adapter_sources(&root, &mut found);

    let expected = root.join("vecdb-core/src/parsers/vecq_adapter.rs");

    assert!(
        found.contains(&expected),
        "the shared adapter is missing from {}. Found instead: {:#?}",
        expected.display(),
        found
    );

    assert_eq!(
        found.len(),
        1,
        "found {} copies of vecq_adapter.rs. There must be exactly one, in \
         vecdb-core, shared by both binaries — the last time there were two they \
         drifted into keying chunk IDs differently, and every MCP re-ingest \
         duplicated the file being ingested.\n\nCopies:\n{:#?}",
        found.len(),
        found
    );
}

/// The chunk ID must be derived from content, never from position.
///
/// A position-keyed ID changes when unrelated lines move, which turns a
/// one-function edit into a whole-file rewrite. This is the specific divergence
/// the server's copy had, so it is pinned by name rather than left to the
/// no-duplicates rule alone.
#[test]
fn chunk_ids_are_content_addressed_not_line_addressed() {
    let source =
        std::fs::read_to_string(workspace_root().join("vecdb-core/src/parsers/vecq_adapter.rs"))
            .expect("shared adapter must be readable");

    // Isolate the seed expression rather than scanning the whole file: line
    // numbers legitimately appear elsewhere, as chunk *metadata*.
    let seed_line = source
        .lines()
        .find(|l| l.contains("chunk_seed"))
        .expect("adapter must build a chunk_seed");

    assert!(
        seed_line.contains("content_hash"),
        "the chunk ID seed must include a hash of the content, so that editing a \
         function changes its ID and re-ingest replaces it.\n  found: {}",
        seed_line.trim()
    );

    assert!(
        !seed_line.contains("line_start"),
        "the chunk ID seed must not include line_start: inserting a line at the \
         top of a file would re-identify every chunk below it and duplicate the \
         whole file on re-ingest.\n  found: {}",
        seed_line.trim()
    );
}
