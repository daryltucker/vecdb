//! The single bridge from `vecq`'s AST elements to vecdb chunks.
//!
//! This lived in two places — `the CLI` and
//! `the server` — and the copies drifted. The server's
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

    /// Pack consecutive sibling elements together until a chunk reaches this
    /// many bytes.
    ///
    /// The AST says where a cut is ALLOWED, not how much belongs in a chunk.
    /// Without this the two questions collapse into one and every element
    /// becomes its own vector, which is how `code` came to have a median chunk
    /// of 39 bytes while `target_chunk_size` sat at 6000 tokens, never once
    /// reached (BUG_CHUNK_GRANULARITY_SUBTOKEN_ELEMENTS-2026-237).
    ///
    /// An element already at or above this size is never split — that is the
    /// AST doing its real job, and cutting a function in half to hit a byte
    /// count would be worse than a large chunk.
    pub pack_target_bytes: usize,

    /// Never emit a chunk below this, unless it is all the file contains — or
    /// its only neighbours are elements that must stand alone.
    ///
    /// A floor, not a filter: undersized content is packed into its neighbours
    /// rather than dropped, so `FIXME: leaks on retry` stays retrievable
    /// instead of being discarded by a threshold.
    ///
    /// Enforced by [`VecqParserAdapter::absorb_undersized`], which documents
    /// what went wrong when this field existed but nothing read it.
    pub min_chunk_bytes: usize,
}

impl Default for VecqAdapterConfig {
    fn default() -> Self {
        Self {
            skip_redundant_parents: true,
            // Deliberately conservative relative to the embedders in play
            // (nomic 8192 tok, qwen 8192 tok): this is a PACKING target, and
            // overshooting it costs retrieval precision, while undershooting
            // only costs vector count. `ingest` overrides it from the resolved
            // chunking config; this default governs the non-ingest paths.
            pack_target_bytes: 2048,
            min_chunk_bytes: 192,
        }
    }
}

/// Element types that are spans INSIDE a block, not blocks themselves.
///
/// These are never chunk candidates. The reason is duplication, not size: a
/// markdown `paragraph` is indexed, and its `strong` child is only ~10% of the
/// parent's bytes so the redundant-parent filter keeps the paragraph too. The
/// same bytes were being embedded twice, once with the surrounding sentence
/// and once as a bare fragment — and the bare fragment competes for top-k
/// against the version that actually carries context.
///
/// vecq is right to emit them; `.strong[]` is a legitimate structural query.
/// Being an element and being worth embedding are different questions.
fn is_inline_span(element_type: &str) -> bool {
    matches!(
        element_type,
        "strong"
            | "emphasis"
            | "strikethrough"
            | "horizontal_rule"
            | "link"
            | "image"
            | "function_call"
            | "variable_reference"
            | "type_reference"
            | "method_call"
            | "assignment"
            | "import_usage"
    )
}

/// Text density, measured the way cAST measures it: non-whitespace characters.
///
/// Raw byte length is the wrong ruler for a packing target. Deeply-indented
/// Rust carries far more whitespace per unit of meaning than flat markdown, so
/// counting bytes systematically under-fills code chunks relative to prose for
/// the same configured target. Counting only non-whitespace keeps one target
/// meaningful across every language and indentation style.
/// (cAST, Findings of EMNLP 2025, arXiv:2506.15655.)
fn density(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

/// A chunk candidate: one element that survived the inline and redundancy
/// filters, carrying what packing needs to decide whether it may merge.
struct Candidate {
    content: String,
    element_type: String,
    trail: String,
    /// Trail of the parent, so only true siblings are packed together.
    parent_trail: String,
    line_start: usize,
    line_end: usize,
    metadata: serde_json::Map<String, serde_json::Value>,
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

    /// Walk the AST and collect chunk CANDIDATES in document order.
    ///
    /// Deliberately does not build chunks or IDs — `pack` does, because the ID
    /// seeds on the final packed content. Splitting the walk from the sizing is
    /// the whole point: the AST answers "what is here and where may I cut", not
    /// "how much belongs in one vector".
    fn flatten_elements(
        &self,
        elements: &[DocumentElement],
        chunks: &mut Vec<Candidate>,
        path: &Path,
        base_metadata: &serde_json::Value,
        parent_trail: &str,
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

            // Inline spans are never candidates — see `is_inline_span`.
            let element_type = element.element_type.to_string();
            if should_index && !is_inline_span(&element_type) && !element.content.trim().is_empty()
            {
                chunks.push(Candidate {
                    content: element.content.clone(),
                    element_type,
                    trail: current_trail.clone(),
                    parent_trail: parent_trail.to_string(),
                    line_start: element.line_start,
                    line_end: element.line_end,
                    metadata,
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
                );
            }
        }
    }

    /// Pack candidates in document order into chunks of roughly
    /// `pack_target_bytes`.
    ///
    /// Only true siblings merge — a run is broken whenever `parent_trail`
    /// changes — so a chunk never straddles two functions or two top-level
    /// sections. An element already at or over the target stands alone.
    fn pack(&self, candidates: Vec<Candidate>, doc_id: &str) -> Vec<Chunk> {
        let mut chunks = Vec::new();
        let mut run: Vec<Candidate> = Vec::new();

        let flush = |run: &mut Vec<Candidate>, chunks: &mut Vec<Chunk>| {
            if run.is_empty() {
                return;
            }
            let content = run
                .iter()
                .map(|c| c.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");

            // Metadata comes from the first member, which is the outermost /
            // earliest element in the run and therefore the one a reader would
            // name the chunk by. The span covers the whole run.
            let first = &run[0];
            let mut metadata = first.metadata.clone();
            let trail = first.trail.clone();
            let line_start = first.line_start;
            let line_end = run.iter().map(|c| c.line_end).max().unwrap_or(line_start);

            metadata.insert("line_end".to_string(), serde_json::json!(line_end));
            if run.len() > 1 {
                // Honest about what this chunk is: a packed run, not a single
                // named declaration. A reader filtering on element_type must
                // not be told this is a bare `paragraph`.
                metadata.insert("packed_elements".to_string(), serde_json::json!(run.len()));
                metadata.insert(
                    "packed_element_types".to_string(),
                    serde_json::json!(run
                        .iter()
                        .map(|c| c.element_type.clone())
                        .collect::<Vec<_>>()),
                );
                // A packed run has no single name.
                metadata.remove("name");
            }

            let content_hash = calculate_hash(&content);
            let chunk_seed = format!("{}::{}::{}", doc_id, trail, content_hash);
            let chunk_id =
                Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, chunk_seed.as_bytes()).to_string();

            chunks.push(Chunk {
                id: chunk_id,
                document_id: doc_id.to_string(),
                content: content.clone(),
                vector: None,
                metadata: metadata.into_iter().collect(),
                page_num: None,
                byte_start: 0,
                byte_end: content.len(),
                start_line: Some(line_start),
                end_line: Some(line_end),
            });
            run.clear();
        };

        // A file smaller than the target is ONE chunk, unconditionally.
        //
        // Splitting it cannot improve retrieval — every piece would be returned
        // for the same queries — and it costs the reader the surrounding
        // context. Size was only ever a proxy for the real question; below the
        // target there is no question to ask. Without this, a 600-byte file
        // with two top-level sections still came out as two chunks, because the
        // run breaks on a parent-trail change.
        let whole_file: usize = candidates.iter().map(|c| density(&c.content)).sum();
        if whole_file <= self.config.pack_target_bytes && !candidates.is_empty() {
            let mut all = candidates;
            flush(&mut all, &mut chunks);
            return chunks;
        }

        // Phase 1 — group into runs. The break rules are unchanged; what used to
        // happen here directly is now deferred so the floor (phase 2) can see a
        // whole run before it becomes a chunk.
        //
        // The bool is "this run stands alone": a single element already at or
        // over the target, which must neither be split nor glued to a neighbour.
        let mut runs: Vec<(Vec<Candidate>, bool)> = Vec::new();
        for cand in candidates {
            let run_len: usize = run.iter().map(|c| density(&c.content)).sum();
            let cand_len = density(&cand.content);

            let breaks_sibling_run = run
                .first()
                .map(|f| f.parent_trail != cand.parent_trail)
                .unwrap_or(false);
            let would_overflow = run_len + cand_len > self.config.pack_target_bytes;
            let stands_alone = cand_len >= self.config.pack_target_bytes;

            if !run.is_empty() && (breaks_sibling_run || would_overflow || stands_alone) {
                runs.push((std::mem::take(&mut run), false));
            }

            if stands_alone {
                runs.push((vec![cand], true));
            } else {
                run.push(cand);
            }
        }
        if !run.is_empty() {
            runs.push((run, false));
        }

        // Phase 2 — apply the floor, then emit.
        for (mut r, _) in self.absorb_undersized(runs) {
            flush(&mut r, &mut chunks);
        }

        chunks
    }

    /// Fold runs below `min_chunk_bytes` into an adjacent run.
    ///
    /// **The floor is a floor, not a filter.** Undersized content is never
    /// dropped — it rides along with the element next to it, so
    /// `FIXME: leaks on retry` stays retrievable instead of being discarded by
    /// a threshold.
    ///
    /// Without this, a `parent_trail` change flushes a run of any size. Deeply
    /// nested data (JSON especially) changes parent at nearly every element, so
    /// the run is flushed while it still holds a few bytes. Source code does
    /// not show it: its parent trails are stable, which is why the other tests
    /// in this module pass either way.
    ///
    /// Two rules constrain the merge:
    ///
    /// - **A stands-alone run is never absorbed and never absorbs.** An element
    ///   already at or over the target must not be glued to a neighbour — see
    ///   `oversized_element_stands_alone`. Accepted consequence: a fragment
    ///   whose only neighbours are stands-alone elements is emitted below the
    ///   floor, because the alternative breaks a rule that matters more.
    /// - **A run that has already reached the target stops accumulating**, or a
    ///   long tail of fragments folds into one chunk and overshoots without
    ///   bound.
    fn absorb_undersized(&self, runs: Vec<(Vec<Candidate>, bool)>) -> Vec<(Vec<Candidate>, bool)> {
        let floor = self.config.min_chunk_bytes;
        let target = self.config.pack_target_bytes;
        let len_of = |r: &Vec<Candidate>| r.iter().map(|c| density(&c.content)).sum::<usize>();

        let mut out: Vec<(Vec<Candidate>, bool)> = Vec::new();
        for (run, solo) in runs {
            if !solo {
                if let Some((prev, prev_solo)) = out.last_mut() {
                    let prev_len = len_of(prev);
                    if !*prev_solo
                        && (len_of(&run) < floor || prev_len < floor)
                        && prev_len < target
                    {
                        prev.extend(run);
                        continue;
                    }
                }
            }
            out.push((run, solo));
        }

        // A trailing fragment has no successor to ride with. Fold it backwards,
        // accepting an overshoot of at most `floor`, rather than emit it alone.
        if out.len() > 1 {
            let trailing_fragment = out
                .last()
                .map(|(r, solo)| !*solo && len_of(r) < floor)
                .unwrap_or(false);
            if trailing_fragment {
                let (frag, _) = out.pop().expect("checked non-empty above");
                match out.last_mut() {
                    Some((prev, prev_solo)) if !*prev_solo => prev.extend(frag),
                    // Its only neighbour stands alone; emit it rather than
                    // corrupt that element. See the doc comment.
                    _ => out.push((frag, false)),
                }
            }
        }

        out
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

        let mut candidates = Vec::new();
        let base_meta = base_metadata.unwrap_or(serde_json::json!({}));

        // Two passes, because they answer two different questions. The first
        // asks the AST "what is here, and where may I cut?"; the second asks
        // "how much belongs in one vector?". Collapsing them is the bug this
        // structure exists to prevent.
        self.flatten_elements(&parsed_doc.elements, &mut candidates, path, &base_meta, "");

        Ok(self.pack(candidates, &doc_id))
    }

    fn supported_extensions(&self) -> Vec<&str> {
        // This adapter is generic, supported extensions are handled by the factory
        vec![]
    }
}

/// Factory that produces vecq parsers
#[derive(Debug, Clone, Copy, Default)]
pub struct VecqParserFactory {
    /// Packing target handed to every adapter this factory builds.
    ///
    /// `Default` leaves it `None`, which means the adapter's own default. The
    /// ingest paths override it from `ingestion.pack_target_bytes`, because
    /// that is the only place the operator's chunking config is resolved —
    /// and packing size is a property of the corpus being built, not of the
    /// binary doing the building.
    pack_target_bytes: Option<usize>,
}

impl VecqParserFactory {
    /// Build a factory whose adapters pack to `bytes`.
    pub fn with_pack_target(bytes: usize) -> Self {
        Self {
            pack_target_bytes: Some(bytes),
        }
    }

    fn adapter_config(&self) -> VecqAdapterConfig {
        let mut cfg = VecqAdapterConfig::default();
        if let Some(b) = self.pack_target_bytes {
            cfg.pack_target_bytes = b;
        }
        cfg
    }
}

impl ParserFactory for VecqParserFactory {
    /// vecq parses everything it supports. There is no second opinion.
    ///
    /// This used to be a `match` whose arms all returned the same value, wrapped
    /// in a paragraph of reasoning about whether to chain `BuiltinParserFactory`
    /// for JSON and TOML. The reasoning never resolved and the code never
    /// branched — which was the right outcome reached by accident, and is now
    /// stated on purpose: one implementation per file type, and it is vecq's.
    fn get_parser(&self, file_type: FileType) -> Option<Box<dyn Parser>> {
        self.get_parser_sized(file_type, None)
    }

    /// The destination's granularity wins over the run's.
    fn get_parser_sized(
        &self,
        file_type: FileType,
        pack_target_bytes: Option<usize>,
    ) -> Option<Box<dyn Parser>> {
        file_type.is_supported().then(|| {
            let mut cfg = self.adapter_config();
            if let Some(b) = pack_target_bytes {
                cfg.pack_target_bytes = b;
            }
            Box::new(VecqParserAdapter::with_config(file_type, cfg)) as Box<dyn Parser>
        })
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

    /// Candidates only — packing is deliberately NOT applied here, so this
    /// helper keeps testing the redundancy filter in isolation rather than
    /// measuring what the packer later merges.
    fn flatten_with(config: VecqAdapterConfig) -> Vec<Candidate> {
        let adapter = VecqParserAdapter::with_config(FileType::Rust, config);
        let mut chunks = Vec::new();
        adapter.flatten_elements(
            &redundant_parent_tree(),
            &mut chunks,
            Path::new("knob.rs"),
            &serde_json::json!({}),
            "",
        );
        chunks
    }

    use vecq::types::{DocumentElement as DocEl, ElementType};

    // ── Packing (BUG_CHUNK_GRANULARITY_SUBTOKEN_ELEMENTS-2026-237) ──────────
    //
    // Before packing existed, every AST element became its own vector: on the
    // live `code` collection the median chunk was 39 characters, 57% were under
    // 50, and the smallest was 1 character — while `target_chunk_size` sat at
    // 6000 tokens and was never once reached. These tests pin the three rules
    // that fixed it. Each one fails against the unpacked adapter.

    /// Build `n` sibling paragraphs of `body_len` non-whitespace characters each.
    fn siblings(n: usize, body_len: usize) -> Vec<DocEl> {
        (0..n)
            .map(|i| {
                DocEl::new(
                    ElementType::Paragraph,
                    Some(format!("p{i}")),
                    "x".repeat(body_len),
                    i + 1,
                    i + 1,
                )
            })
            .collect()
    }

    fn pack_with(config: VecqAdapterConfig, tree: Vec<DocEl>) -> Vec<Chunk> {
        let adapter = VecqParserAdapter::with_config(FileType::Markdown, config);
        let mut cands = Vec::new();
        adapter.flatten_elements(
            &tree,
            &mut cands,
            Path::new("pack.md"),
            &serde_json::json!({}),
            "",
        );
        adapter.pack(cands, "doc-id")
    }

    /// Small siblings must MERGE, not each become a vector.
    ///
    /// 40 paragraphs of 100 chars against a 1000-char target is 4000 chars of
    /// content: roughly 4 chunks, not 40. Unpacked, this returns 40.
    #[test]
    fn small_siblings_are_packed_not_emitted_individually() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 1000,
            ..VecqAdapterConfig::default()
        };
        let chunks = pack_with(cfg, siblings(40, 100));

        assert!(
            chunks.len() <= 6,
            "40 x 100-char siblings against a 1000-char target must pack into a \
             handful of chunks, got {}",
            chunks.len()
        );
        let smallest = chunks.iter().map(|c| density(&c.content)).min().unwrap();
        assert!(
            smallest >= 500,
            "no chunk should be a fragment; smallest carried {smallest} non-whitespace chars"
        );
    }

    /// A changing parent trail must NOT flush a run that is still under the
    /// floor.
    ///
    /// Deeply nested data changes parent at nearly every element, so the
    /// sibling-run break fires constantly and emits whatever has accumulated —
    /// a handful of bytes at a time. Source code never shows this, because its
    /// parent trails are stable; this reproduces the nested-data shape
    /// directly.
    ///
    /// Without the floor, this returns ~60 chunks of ~60 characters and the
    /// assertion below fails on the first one.
    #[test]
    fn changing_parent_trail_does_not_emit_below_the_floor() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 2048,
            min_chunk_bytes: 192,
            ..VecqAdapterConfig::default()
        };

        let chunks = pack_with(cfg, nested_sections());
        assert!(!chunks.is_empty(), "packing produced nothing");

        // Guard against this test going vacuous: with too little content the
        // whole-file rule collapses everything to one chunk and the assertion
        // below cannot fail no matter what `pack()` does.
        assert!(
            chunks.len() > 1,
            "fixture is too small — the whole-file rule produced one chunk, so \
             this test exercises nothing. Add sections."
        );

        let smallest = chunks.iter().map(|c| density(&c.content)).min().unwrap();
        assert!(
            smallest >= 192,
            "a parent-trail change flushed a run below min_chunk_bytes: smallest \
             chunk carried {smallest} non-whitespace chars across {} chunks.",
            chunks.len()
        );

        // And the corollary: content is never dropped to satisfy the floor.
        let packed: usize = chunks.iter().map(|c| density(&c.content)).sum();
        let unpacked: usize = pack_with(
            VecqAdapterConfig {
                pack_target_bytes: 1,
                min_chunk_bytes: 0,
                ..VecqAdapterConfig::default()
            },
            nested_sections(),
        )
        .iter()
        .map(|c| density(&c.content))
        .sum();
        assert_eq!(
            packed, unpacked,
            "the floor must merge undersized runs, never discard them"
        );
    }

    /// 60 headers, each owning one small paragraph: every paragraph sits under
    /// a different parent, so every element breaks the sibling run. Sized so
    /// total density clears `pack_target_bytes` — otherwise the whole-file rule
    /// collapses it to one chunk and the floor is never exercised.
    fn nested_sections() -> Vec<DocEl> {
        (0..60)
            .map(|i| {
                DocEl::new(
                    ElementType::Header,
                    Some(format!("h{i}")),
                    format!("# Section {i}"),
                    i * 2 + 1,
                    i * 2 + 1,
                )
                .with_children(vec![DocEl::new(
                    ElementType::Paragraph,
                    None,
                    format!("body of section {i} ").repeat(4),
                    i * 2 + 2,
                    i * 2 + 2,
                )])
            })
            .collect()
    }

    /// An element already at or over the target stands alone — a function is
    /// never cut in half to hit a byte count, and never glued to a neighbour.
    #[test]
    fn oversized_element_stands_alone() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 100,
            ..VecqAdapterConfig::default()
        };
        let mut tree = siblings(2, 10);
        tree.insert(
            1,
            DocEl::new(
                ElementType::Function,
                Some("big".to_string()),
                "y".repeat(5000),
                2,
                2,
            ),
        );
        let chunks = pack_with(cfg, tree);

        let big: Vec<_> = chunks
            .iter()
            .filter(|c| density(&c.content) >= 5000)
            .collect();
        assert_eq!(
            big.len(),
            1,
            "the oversized element must survive as one chunk"
        );
        assert_eq!(
            density(&big[0].content),
            5000,
            "it must not absorb its small neighbours"
        );
    }

    /// A file below the target is ONE chunk, even when its elements sit under
    /// different parents. Splitting it cannot improve retrieval — every piece
    /// answers the same queries — and it costs the reader the surrounding
    /// context.
    #[test]
    fn file_under_target_is_a_single_chunk() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 2048,
            ..VecqAdapterConfig::default()
        };
        let a = DocEl::new(
            ElementType::Header,
            Some("one".to_string()),
            "# One".to_string(),
            1,
            1,
        )
        .with_children(vec![DocEl::new(
            ElementType::Paragraph,
            None,
            "first section body".to_string(),
            2,
            2,
        )]);
        let b = DocEl::new(
            ElementType::Header,
            Some("two".to_string()),
            "# Two".to_string(),
            3,
            3,
        )
        .with_children(vec![DocEl::new(
            ElementType::Paragraph,
            None,
            "second section body".to_string(),
            4,
            4,
        )]);

        let chunks = pack_with(cfg, vec![a, b]);
        assert_eq!(
            chunks.len(),
            1,
            "a small file must not be split across parent-trail boundaries"
        );
    }

    /// Inline spans are not chunk candidates. The reason is duplication, not
    /// size: the parent paragraph is already indexed, so emitting the span
    /// separately puts the same bytes in the index twice and the bare fragment
    /// competes for top-k against the version carrying context.
    #[test]
    fn inline_spans_are_not_chunked() {
        let para = DocEl::new(
            ElementType::Paragraph,
            None,
            "The retry policy is transient only.".to_string(),
            1,
            1,
        )
        .with_children(vec![DocEl::new(
            ElementType::Strong,
            None,
            "transient".to_string(),
            1,
            1,
        )]);

        let adapter = VecqParserAdapter::new(FileType::Markdown);
        let mut cands = Vec::new();
        adapter.flatten_elements(
            &[para],
            &mut cands,
            Path::new("inline.md"),
            &serde_json::json!({}),
            "",
        );
        assert!(
            cands.iter().all(|c| c.element_type != "strong"),
            "a `strong` span must never become a chunk candidate"
        );
    }

    /// Size is counted in non-whitespace characters, so indentation does not
    /// change how much meaning a chunk carries (cAST, arXiv:2506.15655).
    #[test]
    fn density_ignores_whitespace() {
        assert_eq!(density("        fn a() {}"), density("fn a() {}"));
        assert_eq!(density(" \n\t "), 0);
    }

    /// The knob must be load-bearing, not decorative. With the filter ON the
    /// fully-covered non-declaration parent is dropped; with it OFF the parent
    /// is emitted — which is precisely what the stale server copy did, inflating
    /// collections with redundant parent chunks.
    #[test]
    fn skip_redundant_parents_knob_changes_output() {
        let filtered = flatten_with(VecqAdapterConfig {
            skip_redundant_parents: true,
            ..VecqAdapterConfig::default()
        });
        let unfiltered = flatten_with(VecqAdapterConfig {
            skip_redundant_parents: false,
            ..VecqAdapterConfig::default()
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

        let trail_of = |c: &Candidate| c.trail.clone();
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
