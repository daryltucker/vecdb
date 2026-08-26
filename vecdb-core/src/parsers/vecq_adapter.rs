//! The single bridge from `vecq`'s AST elements to vecdb chunks.
//!
//! This lived in two places — `vecdb-cli/src/vecq_adapter.rs` and
//! `vecdb-server/src/vecq_adapter.rs` — and the copies drifted. The server's
//! keyed chunk IDs on `line_start` instead of a content hash, so inserting a
//! line at the top of a file re-identified every chunk below it and re-ingest
//! duplicated the file; it also lacked the parent-redundancy filter and the
//! streaming JSON parser. Ivaldi drives the MCP server, so Ivaldi got all three.
//!
//! It lives in `vecdb-core` so that both binaries share one implementation and
//! there is no second copy to drift. An earlier rule forbade `vecdb-core` from
//! depending on `vecq`; pushing the adapter out to satisfy it is what produced
//! the two copies in the first place. That rule has since been retired — see
//! `tier1_architecture.rs`.

use crate::parsers::{Parser, ParserFactory};
use crate::types::Chunk;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;
use uuid::Uuid;
use vecdb_common::FileType;
use vecq::DocumentElement;

/// Tunable behaviour for [`VecqParserAdapter`].
///
/// Any legitimate difference between the CLI and MCP ingestion paths MUST be
/// expressed here as a field, never as a duplicated module. This adapter
/// previously existed as two byte-divergent copies — `vecdb-cli`'s (correct)
/// and `vecdb-server`'s (a pre-2026-211 fossil that seeded chunk IDs with
/// `line_start`), so the same file ingested through the two paths produced
/// different UUIDs and re-ingestion DUPLICATED rather than deduplicated.
///
/// Today both callers use [`VecqAdapterConfig::default`]. The struct exists so
/// that a future divergence has a home that cannot silently rot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VecqAdapterConfig {
    /// Skip emitting a chunk for a parent element whose content is already
    /// almost entirely covered by its children (>90%), unless that parent is a
    /// semantically meaningful declaration or carries a docstring.
    ///
    /// Historically only the CLI copy applied this filter; the server copy
    /// indexed every element, inflating collections with redundant parents.
    pub skip_redundant_parents: bool,
}

impl Default for VecqAdapterConfig {
    fn default() -> Self {
        Self {
            skip_redundant_parents: true,
        }
    }
}

/// Adapter to use vecq parsers within vecdb ingestion
pub struct VecqParserAdapter {
    file_type: FileType,
    config: VecqAdapterConfig,
}

impl VecqParserAdapter {
    pub fn new(file_type: FileType) -> Self {
        Self {
            file_type,
            config: VecqAdapterConfig::default(),
        }
    }

    /// Construct an adapter with non-default behaviour.
    pub fn with_config(file_type: FileType, config: VecqAdapterConfig) -> Self {
        Self { file_type, config }
    }

    /// Recursively flatten vecq elements into chunks
    fn flatten_elements(
        &self,
        elements: &[DocumentElement],
        chunks: &mut Vec<Chunk>,
        path: &Path,
        base_metadata: &serde_json::Value,
        parent_trail: &str,
        doc_id: &str,
    ) {
        for element in elements {
            // Basic metadata
            let mut metadata = base_metadata.as_object().cloned().unwrap_or_default();
            metadata.insert(
                "element_type".to_string(),
                serde_json::Value::String(element.element_type.to_string()),
            );
            if let Some(name) = &element.name {
                metadata.insert("name".to_string(), serde_json::Value::String(name.clone()));
            }
            metadata.insert(
                "line_start".to_string(),
                serde_json::json!(element.line_start),
            );
            metadata.insert("line_end".to_string(), serde_json::json!(element.line_end));
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String(path.to_string_lossy().to_string()),
            );
            metadata.insert(
                "file_type".to_string(),
                serde_json::Value::String(self.file_type.to_string()),
            );

            // Extract Semantic Intent (Phase 3)
            if let Some(doc) = element.attributes.get("docstring") {
                metadata.insert("docstring".to_string(), doc.clone());
                metadata.insert("intent".to_string(), doc.clone()); // Alias for semantic alignment
            }
            if let Some(vis) = element.attributes.get("visibility") {
                metadata.insert("visibility".to_string(), vis.clone());
            }

            // Crumbtrail (Phase 1)
            let current_trail = if parent_trail.is_empty() {
                element
                    .name
                    .clone()
                    .unwrap_or(element.element_type.to_string())
            } else {
                format!(
                    "{}::{}",
                    parent_trail,
                    element
                        .name
                        .as_deref()
                        .unwrap_or(&element.element_type.to_string())
                )
            };
            metadata.insert(
                "crumbtrail".to_string(),
                serde_json::Value::String(current_trail.clone()),
            );

            // Redundancy Check: If it has children, only add it if it has "meat" (unique content)
            let children_len: usize = element.children.iter().map(|c| c.content.len()).sum();
            let is_fully_covered =
                !element.children.is_empty() && (children_len > (element.content.len() * 9 / 10));

            let should_index = if is_fully_covered && self.config.skip_redundant_parents {
                matches!(
                    element.element_type.to_string().as_str(),
                    "function" | "method" | "class" | "struct" | "interface" | "trait"
                ) || element.attributes.contains_key("docstring")
            } else {
                true
            };

            if should_index {
                // Create deterministic ID based on doc ID + crumbtrail + content hash for maximum stability
                let content_hash = calculate_hash(&element.content);
                let chunk_seed = format!("{}::{}::{}", doc_id, current_trail, content_hash);
                let chunk_id =
                    Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, chunk_seed.as_bytes()).to_string();

                chunks.push(Chunk {
                    id: chunk_id,
                    document_id: doc_id.to_string(),
                    content: element.content.clone(),
                    vector: None,
                    metadata: metadata.into_iter().collect(),
                    page_num: None,
                    byte_start: 0,
                    byte_end: element.content.len(),
                    start_line: Some(element.line_start),
                    end_line: Some(element.line_end),
                });
            }

            // Recurse
            if !element.children.is_empty() {
                self.flatten_elements(
                    &element.children,
                    chunks,
                    path,
                    base_metadata,
                    &current_trail,
                    doc_id,
                );
            }
        }
    }
}

fn calculate_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    result
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

#[async_trait]
impl Parser for VecqParserAdapter {
    async fn parse(
        &self,
        content: &str,
        path: &Path,
        base_metadata: Option<serde_json::Value>,
    ) -> Result<Vec<Chunk>> {
        // Use vecq to parse the file
        // Note: vecq::parse_file takes &str content and FileType
        let parsed_doc = vecq::parse_file(content, self.file_type).await?;

        // Generate a document ID (could be from file path hash)
        // Here we can use file path + optional commit sha from metadata
        let doc_seed = path.to_string_lossy().to_string();
        let doc_id = Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, doc_seed.as_bytes()).to_string();

        let mut chunks = Vec::new();
        let base_meta = base_metadata.unwrap_or(serde_json::json!({}));

        // Flatten the parsed document hierarchy into a list of chunks
        self.flatten_elements(
            &parsed_doc.elements,
            &mut chunks,
            path,
            &base_meta,
            "",
            &doc_id,
        );

        Ok(chunks)
    }

    fn supported_extensions(&self) -> Vec<&str> {
        // This adapter is generic, supported extensions are handled by the factory
        vec![]
    }
}

/// Factory that produces vecq parsers
pub struct VecqParserFactory;

impl ParserFactory for VecqParserFactory {
    /// vecq parses everything it supports. There is no second opinion.
    ///
    /// This used to be a `match` whose arms all returned the same value, wrapped
    /// in a paragraph of reasoning about whether to chain `BuiltinParserFactory`
    /// for JSON and TOML. The reasoning never resolved and the code never
    /// branched — which was the right outcome reached by accident, and is now
    /// stated on purpose: one implementation per file type, and it is vecq's.
    fn get_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        file_type
            .is_supported()
            .then(|| Box::new(VecqParserAdapter::new(file_type)) as Box<dyn Parser>)
    }

    /// The one deliberate exception, and it is about memory rather than meaning.
    ///
    /// vecq parses to a full AST in memory, which a multi-gigabyte JSON export
    /// will not survive. Files past `LARGE_FILE_THRESHOLD` therefore take
    /// `StreamingJsonParser` instead. That does mean JSON has two
    /// implementations selected by file size, so `tier1_parser_authority.rs`
    /// pins this as the *only* such split — the general rule is unchanged.
    fn get_streaming_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        match file_type {
            FileType::Json => Some(Box::new(
                crate::parsers::streaming_json::StreamingJsonParser::new(),
            )),
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Two `pub fn` declarations, deliberately trivial so the parse is stable.
    const SRC: &str = "pub fn alpha() -> u32 {\n    1\n}\n\npub fn beta() -> u32 {\n    2\n}\n";

    async fn ids_for(src: &str) -> Vec<String> {
        let adapter = VecqParserAdapter::new(FileType::Rust);
        let chunks = adapter
            .parse(src, Path::new("probe.rs"), None)
            .await
            .expect("vecq must parse trivial Rust");
        assert!(!chunks.is_empty(), "fixture produced no chunks");
        chunks.into_iter().map(|c| c.id).collect()
    }

    /// REGRESSION (WORK_LOG-2026-211 / -220 §C-1).
    ///
    /// Shifting every element down by two blank lines changes `line_start` for
    /// every chunk but changes no content. Chunk IDs MUST be unaffected.
    ///
    /// This is the exact property the stale `vecdb-server` copy violated: it
    /// seeded on `element.line_start`, so inserting one line at the top re-IDed
    /// every chunk below it and re-ingestion duplicated instead of deduplicating.
    #[tokio::test]
    async fn chunk_ids_are_invariant_under_line_shift() {
        let base = ids_for(SRC).await;
        let shifted = ids_for(&format!("\n\n{}", SRC)).await;

        assert_eq!(
            base.len(),
            shifted.len(),
            "line shift must not change chunk count"
        );
        assert_eq!(
            base, shifted,
            "chunk IDs changed under a pure line shift — the chunk seed has \
             regressed to incorporating line numbers (see WORK_LOG-2026-211)"
        );
    }

    /// Guard against a degenerate always-equal implementation: different
    /// content MUST yield different IDs.
    #[tokio::test]
    async fn chunk_ids_change_when_content_changes() {
        let base = ids_for(SRC).await;
        let mutated = ids_for(&SRC.replace("    1", "    999")).await;

        assert_eq!(
            base.len(),
            mutated.len(),
            "fixture shape changed unexpectedly"
        );
        assert_ne!(
            base, mutated,
            "content changed but chunk IDs did not — the ID is not content-addressable"
        );
    }

    /// Pin the seed format itself so a future refactor cannot quietly change it
    /// (which would orphan every already-ingested vector).
    ///
    /// Expected value derived from the pre-move `vecdb-cli` implementation:
    ///   seed = "{doc_id}::{crumbtrail}::{sha256(content)}"
    ///   id   = UUIDv5(NAMESPACE_URL, seed)
    #[test]
    fn chunk_seed_format_is_pinned() {
        let doc_id = Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, "probe.rs".as_bytes()).to_string();
        let content = "pub fn alpha() -> u32 {\n    1\n}";
        let seed = format!("{}::{}::{}", doc_id, "alpha", calculate_hash(content));
        let id = Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, seed.as_bytes()).to_string();

        assert_eq!(
            doc_id, "5409e66f-6e0b-5a99-9924-a2e39a5cae24",
            "doc_id derivation changed"
        );
        assert_eq!(
            calculate_hash(content),
            "5ede4b8332920b9dcb846775025c4358a7983febe899313a46cd3ce7f240103b",
            "SHA-256 helper changed"
        );
        assert_eq!(
            id, "178d1e90-3b0a-5a53-bfcd-1be3e5c2934e",
            "chunk seed format changed — every already-ingested vector would be orphaned"
        );
    }

    /// The shared `calculate_hash` must be byte-identical to the private helper
    /// it replaced in `vecdb-cli` (`sha2::Sha256` + `format!("{:02x}", b)`).
    #[test]
    fn calculate_hash_matches_legacy_cli_helper() {
        fn legacy(content: &str) -> String {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect::<String>()
        }

        for sample in ["", "a", "pub fn alpha() -> u32 {\n    1\n}", "ünïcödé ✓"] {
            assert_eq!(
                calculate_hash(sample),
                legacy(sample),
                "hash divergence for {sample:?} would orphan existing vectors"
            );
        }
    }

    /// `skip_redundant_parents` is the one behavioural knob that differed
    /// between the two copies. Assert the default matches the CLI (correct)
    /// behaviour and that the knob is actually wired.
    #[test]
    fn default_config_skips_redundant_parents() {
        assert!(VecqAdapterConfig::default().skip_redundant_parents);
    }

    /// Build a parent element whose children cover >90% of its content, and
    /// whose `element_type` is NOT in the meaningful-declaration allowlist, so
    /// the redundancy filter actually engages.
    fn redundant_parent_tree() -> Vec<DocumentElement> {
        use vecq::types::{DocumentElement, ElementType};

        let child_a = DocumentElement::new(
            ElementType::Function,
            Some("a".to_string()),
            "pub fn a() -> u32 { 1 }".to_string(),
            2,
            2,
        );
        let child_b = DocumentElement::new(
            ElementType::Function,
            Some("b".to_string()),
            "pub fn b() -> u32 { 2 }".to_string(),
            3,
            3,
        );
        // Parent content == children concatenated, so coverage is ~100%.
        let parent_content = format!("{}{}", child_a.content, child_b.content);
        let parent = DocumentElement::new(
            ElementType::Block,
            Some("wrapper".to_string()),
            parent_content,
            1,
            4,
        )
        .with_children(vec![child_a, child_b]);

        vec![parent]
    }

    fn flatten_with(config: VecqAdapterConfig) -> Vec<Chunk> {
        let adapter = VecqParserAdapter::with_config(FileType::Rust, config);
        let mut chunks = Vec::new();
        adapter.flatten_elements(
            &redundant_parent_tree(),
            &mut chunks,
            Path::new("knob.rs"),
            &serde_json::json!({}),
            "",
            "doc-id",
        );
        chunks
    }

    /// The knob must be load-bearing, not decorative. With the filter ON the
    /// fully-covered non-declaration parent is dropped; with it OFF the parent
    /// is emitted — which is precisely what the stale server copy did, inflating
    /// collections with redundant parent chunks.
    #[test]
    fn skip_redundant_parents_knob_changes_output() {
        let filtered = flatten_with(VecqAdapterConfig {
            skip_redundant_parents: true,
        });
        let unfiltered = flatten_with(VecqAdapterConfig {
            skip_redundant_parents: false,
        });

        assert_eq!(
            filtered.len(),
            2,
            "filter ON must drop the fully-covered wrapper, keeping only the two functions"
        );
        assert_eq!(
            unfiltered.len(),
            3,
            "filter OFF must emit the wrapper too (legacy server behaviour)"
        );

        let trail_of = |c: &Chunk| {
            c.metadata
                .get("crumbtrail")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        assert!(
            !filtered.iter().any(|c| trail_of(c) == "wrapper"),
            "redundant parent leaked through the filter"
        );
        assert!(
            unfiltered.iter().any(|c| trail_of(c) == "wrapper"),
            "knob had no effect — it is not wired to the redundancy check"
        );
    }
}
