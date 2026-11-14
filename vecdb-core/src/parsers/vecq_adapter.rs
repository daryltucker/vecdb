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
    /// reached.
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

    /// The destination's byte ceiling — the largest chunk that will be
    /// embedded whole, in `String::len()` bytes.
    ///
    /// Answers "does this element FIT?", which is a different question from
    /// "how much belongs in one vector" (`pack_target_bytes`). A JSON record
    /// that fits is emitted whole and never destructured: there is no
    /// duplication to save and every key to lose. Gating that on `pack_target_bytes`
    /// instead is how the fix came out inverted.
    pub max_chunk_bytes: usize,
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
            // The same derivation `flush_chunks` falls back to, so an adapter
            // built without a destination agrees with the pipeline about what
            // fits.
            max_chunk_bytes: crate::config::default_max_chunk_bytes(
                crate::config::DEFAULT_TARGET_CHUNK_SIZE,
            ),
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

/// Element types that are named declarations — a function, method, class,
/// struct, interface, or trait. These are the units of source code that must
/// never be merged with a different named unit, even when they share a parent
/// trail and are below the packing floor.
fn is_named_unit(element_type: &str) -> bool {
    matches!(
        element_type,
        "function" | "method" | "class" | "struct" | "interface" | "trait"
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
pub(crate) fn density(s: &str) -> usize {
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
    /// Label lines this element is read under, outermost first: the Markdown
    /// heading path (`# Doc`, `## Section`), or the key path and identifying
    /// fields of the JSON record it was cut from.
    ///
    /// Written into the chunk TEXT, not only the payload. Only `content` is
    /// embedded, so a label that lives in `crumbtrail` is seen by a human
    /// reading a hit and never by the vector. `pack`
    /// emits each line once per chunk, where it first applies.
    context: Vec<String>,
    /// First block of a Markdown section. Like a named code unit, it is never
    /// packed with, or folded into, what came before it: two sections are two
    /// topics, and a vector averaged over both serves neither. Set by
    /// `flatten_markdown` only; code uses `is_named_unit`.
    unit_start: bool,
    /// A preferred place to cut BEFORE this candidate — a break OPPORTUNITY,
    /// like `<wbr>` or a CSS page-break opportunity (operator, 2026-300): when
    /// the run fits, it changes nothing; when the next candidate would overflow,
    /// the run is cut at the latest opportunity instead of wherever size ran
    /// out. Set by `flatten_markdown` after a thematic break (`---`). Generic:
    /// any walk may mark one.
    break_before: bool,
}

impl Candidate {
    /// Begins a unit that must not merge with its predecessor.
    fn starts_unit(&self) -> bool {
        self.unit_start || is_named_unit(&self.element_type)
    }
}

/// Payload key recording the label `pack` prefixed to a chunk's content.
///
/// `flush_chunks` reads it to repeat the label into every part when it splits
/// an oversized chunk — otherwise part 0 says what it is about and parts 1..n
/// are anonymous text.
pub const CONTEXT_LABEL_KEY: &str = "context_label";

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
        source: &str,
    ) {
        // Spans already embedded at this level. vecq gives one element per
        // NAME of a statement that imports several — Python `from typing import
        // List, Dict`, Go's grouped `import ( … )` — each spanning the whole
        // statement (the span gate allows it: shared spans own nothing). One
        // statement is embedded once; it was embedded once per name (9%
        // duplication on `python/complex.py`, 2026-314).
        let mut embedded: Vec<(usize, usize)> = Vec::new();
        for element in elements {
            if let Some(span) = element.source_span() {
                if embedded.contains(&span) {
                    continue;
                }
                embedded.push(span);
            }
            // Crumbtrail (Phase 1)
            let current_trail = child_trail(parent_trail, element);
            let metadata = self.element_metadata(element, path, base_metadata, &current_trail);

            // Redundancy Check: If it has children, only add it if it has "meat" (unique content)
            //
            // This is the rule for SOURCE CODE, and only for source code. JSON
            // never reaches it: a JSON object's keys are edges, not children,
            // so "children cover 90% of the bytes" there means "dropping the
            // parent discards every key". JSON has its own walk
            // in `flatten_json`, gated on fit rather than on this ratio.
            //
            // The fix once added `&& children_len <= pack_target_bytes` here.
            // That inverted the reported rule — JSON records with SMALL children still
            // lost their keys — and made a large code wrapper keep its body twice.
            let children_len: usize = element.children.iter().map(|c| c.content.len()).sum();
            let is_fully_covered =
                !element.children.is_empty() && (children_len > (element.content.len() * 9 / 10));

            // Superseded for containers by the frame rule below (2026-300):
            // with children and the knob on, only the frame is emitted, which
            // never duplicates — so the ratio must not also drop it. It did,
            // and a C++ namespace lost its header and every comment inside it.
            // The ratio still applies when the knob is off (legacy path).
            let frame_rule = !element.children.is_empty() && self.config.skip_redundant_parents;
            let should_index = if frame_rule {
                true
            } else if is_fully_covered && self.config.skip_redundant_parents {
                matches!(
                    element.element_type.to_string().as_str(),
                    "function" | "method" | "class" | "struct" | "interface" | "trait"
                ) || element.attributes.contains_key("docstring")
            } else {
                true
            };

            // Inline spans are never candidates — see `is_inline_span`.
            let element_type = element.element_type.to_string();

            // Containment, decided once (2026-300): when a container's members
            // are emitted, the container contributes only its FRAME — its own
            // text with the members cut out (`impl Foo {`, a class header, an
            // associated const the walker did not make a child). Emitting the
            // whole container as well embedded every member twice (Rust impl +
            // methods: 26% duplication); dropping it lost the header. Members
            // stay separate chunks: named units never merge.
            // With the knob off, the legacy behaviour (everything) remains.
            let content = if !element.children.is_empty() && self.config.skip_redundant_parents {
                // Exact, from the spans vecq hands over. The text-search fallback
                // runs only where vecq could not supply spans (synthetic trees,
                // rendered content) — never as the primary path.
                element
                    .frame(source)
                    .unwrap_or_else(|| container_frame(&element.content, &element.children))
            } else {
                element.content.clone()
            };
            let meaningful = vecdb_common::has_meaning(&content);
            if should_index && !is_inline_span(&element_type) && meaningful {
                let (content, lines_above) = with_owned_comments(element, &content);
                chunks.push(Candidate {
                    content,
                    element_type,
                    trail: current_trail.clone(),
                    parent_trail: parent_trail.to_string(),
                    line_start: element.line_start.saturating_sub(lines_above),
                    line_end: element.line_end,
                    metadata,
                    context: Vec::new(),
                    unit_start: false,
                    break_before: false,
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
                    source,
                );
            }
        }
    }

    /// The payload every candidate carries, whichever walk produced it.
    fn element_metadata(
        &self,
        element: &DocumentElement,
        path: &Path,
        base_metadata: &serde_json::Value,
        trail: &str,
    ) -> serde_json::Map<String, serde_json::Value> {
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
        metadata.insert(
            "crumbtrail".to_string(),
            serde_json::Value::String(trail.to_string()),
        );
        metadata
    }

    /// The JSON walk: **an element that fits is emitted whole and never
    /// descended.**
    ///
    /// `vecq` gives a JSON object's full serialization as its content and each
    /// value, bare, as a child. Keys are edges, not nodes. So:
    ///
    /// - A container that fits `max_chunk_bytes` is one candidate, and its
    ///   children are NOT also emitted — they are already inside it. Doing both
    ///   is a mistake; dropping the parent for the children is the other error.
    /// - A container that does not fit is descended, and every piece is
    ///   labelled: object members render as `"key": value`, and each piece's
    ///   `context` names the container's key path and its identifying fields
    ///   (see [`json_identity`]), so a fragment still says which record it is.
    /// - A single value too large for any chunk names its own key in its
    ///   context, so `flush_chunks` can repeat it into every split part.
    ///
    /// `vecq` has already descended the document root — its top-level members
    /// arrive as `siblings` — so the root is treated as a descended object.
    /// `identity_allowed` is false where `siblings` are not one object's
    /// members: array items, and JSONL, whose roots' members arrive mixed.
    #[allow(clippy::too_many_arguments)]
    fn flatten_json(
        &self,
        siblings: &[DocumentElement],
        chunks: &mut Vec<Candidate>,
        path: &Path,
        base_metadata: &serde_json::Value,
        parent_trail: &str,
        context: &[String],
        identity_allowed: bool,
        source: &str,
    ) {
        let mut identity = if identity_allowed {
            json_identity(siblings, source)
        } else {
            Vec::new()
        };
        // Identity members ride in the label of the OTHER members' chunks. With
        // no other member there is no chunk to carry them, and the object
        // vanished whole: `{"leaf": "bottom", "n": 0}` or an object of short
        // strings, once too big to keep whole (T1.19 JSON coverage gate, 2026-300).
        if identity.len() == siblings.len() {
            identity.clear();
        }

        let mut ctx = context.to_vec();
        let identity_obj = (!identity.is_empty()).then(|| {
            let fields: Vec<String> = identity
                .iter()
                .map(|&i| json_as_written(&siblings[i], source))
                .collect();
            format!("{{{}}}", fields.join(", "))
        });
        match (parent_trail.is_empty(), identity_obj) {
            (true, None) => {}
            (true, Some(obj)) => ctx.push(obj),
            (false, None) => ctx.push(parent_trail.to_string()),
            (false, Some(obj)) => ctx.push(format!("{parent_trail} {obj}")),
        }
        let ctx_len = context_len(&ctx);

        for (i, element) in siblings.iter().enumerate() {
            // Carried by `ctx` into every sibling's chunk, so never lost —
            // which holds only because a non-identity sibling exists (above).
            if identity.contains(&i) {
                continue;
            }
            let trail = child_trail(parent_trail, element);
            let metadata = self.element_metadata(element, path, base_metadata, &trail);

            if !element.children.is_empty() {
                let whole = render_json_member(element, source);
                if ctx_len + whole.len() <= self.config.max_chunk_bytes {
                    chunks.push(Candidate {
                        content: whole,
                        element_type: element.element_type.to_string(),
                        trail,
                        parent_trail: parent_trail.to_string(),
                        line_start: element.line_start,
                        line_end: element.line_end,
                        metadata,
                        context: ctx.clone(),
                        unit_start: false,
                        break_before: false,
                    });
                } else {
                    // Descended, so it has children; an array's are named
                    // `[i]`. Not `content`'s first byte, which is the key's quote
                    // for every member.
                    let is_object = !element.children.first().is_some_and(is_json_array_item);
                    let first = chunks.len();
                    self.flatten_json(
                        &element.children,
                        chunks,
                        path,
                        base_metadata,
                        &trail,
                        &ctx,
                        is_object,
                        source,
                    );
                    // The container is not a candidate itself, so its own
                    // comments ride with the first and last pieces cut from it.
                    let (leading, trailing) = json_comments(element);
                    if let (Some(c), Some(head)) = (leading, chunks.get_mut(first)) {
                        head.content = format!("{c}\n{}", head.content);
                    }
                    if let (Some(c), true) = (trailing, chunks.len() > first) {
                        if let Some(tail) = chunks.last_mut() {
                            tail.content = format!("{} {c}", tail.content);
                        }
                    }
                }
                continue;
            }

            if element.content.trim().is_empty() {
                continue;
            }
            let rendered = render_json_member(element, source);
            let (content, context) = if ctx_len + rendered.len() <= self.config.max_chunk_bytes {
                (rendered, ctx.clone())
            } else {
                // It will be split. Name it in the label so every part says
                // which value it is a piece of, not just the first.
                let mut own = ctx.clone();
                own.push(trail.clone());
                (rendered, own)
            };
            chunks.push(Candidate {
                content,
                element_type: element.element_type.to_string(),
                trail,
                parent_trail: parent_trail.to_string(),
                line_start: element.line_start,
                line_end: element.line_end,
                metadata,
                context,
                unit_start: false,
                break_before: false,
            });
        }
    }

    /// The walk for nested documents whose containers hold their members'
    /// text — YAML mappings and sequences, HTML elements: **an element that
    /// fits is embedded whole, as written, and never descended** (the JSON
    /// rule, without JSON's key rendering).
    ///
    /// They took the code walk until 2026-314, which emits a container's
    /// FRAME and then each member: right for `impl Foo { … }` + methods, wrong
    /// here. A YAML file came back re-assembled from fragments — indentation,
    /// `- ` markers, the header comment and `---` gone, `["a", "b"]` as `[]`
    /// then `"a"` then `"b"` (29–34% of tokens lost); HTML `<head><title>`
    /// as `<head></head>` + `<title>…` (22% duplicated). Measured with
    /// `tools/chunks/coverage.py`, gated by T1.26.
    ///
    /// - Whole, the element's text is its source slice plus the trivia it
    ///   owns, non-blank lines kept as written: a comment above a key, the
    ///   `---` between YAML documents, an HTML comment.
    /// - Too big, it is descended, and every piece carries the container's own
    ///   first line (`services:`, `<div class="nav">`) in its `context`, which
    ///   `pack` writes into the chunk text — a fragment still says where it
    ///   is. Members never merge across containers (`parent_trail`).
    #[allow(clippy::too_many_arguments)]
    fn flatten_written(
        &self,
        elements: &[DocumentElement],
        chunks: &mut Vec<Candidate>,
        path: &Path,
        base_metadata: &serde_json::Value,
        parent_trail: &str,
        context: &[String],
        source: &str,
    ) {
        let ctx_len = context_len(context);
        for element in elements {
            let trail = child_trail(parent_trail, element);
            let metadata = self.element_metadata(element, path, base_metadata, &trail);
            let (whole, lines_above) = with_trivia_as_written(element, &element.content);
            if element.children.is_empty() || ctx_len + whole.len() <= self.config.max_chunk_bytes {
                if !vecdb_common::has_meaning(&whole) {
                    continue;
                }
                chunks.push(Candidate {
                    content: whole,
                    element_type: element.element_type.to_string(),
                    trail,
                    parent_trail: parent_trail.to_string(),
                    line_start: element.line_start.saturating_sub(lines_above),
                    line_end: element.line_end,
                    metadata,
                    context: context.to_vec(),
                    unit_start: false,
                    break_before: false,
                });
                continue;
            }
            // Descend. The container's words outside its members — its key, its
            // start tag — become the label of every piece cut from it.
            let frame = element
                .frame(source)
                .unwrap_or_else(|| container_frame(&element.content, &element.children));
            // A container with no words of its own (a YAML document) adds no
            // label: a synthetic name like `document[0]` is not in the file.
            let mut ctx = context.to_vec();
            let mut frame_lines = frame
                .lines()
                .map(str::trim)
                .filter(|l| vecdb_common::has_meaning(l));
            if let Some(label) = frame_lines.next() {
                ctx.push(label.to_string());
            }
            // The rest of the frame — an end tag, text between an HTML
            // element's children — is the container's own content: embedded
            // once, after its members, never only as a label (T1.26 found
            // `</body>` lost).
            let rest: Vec<&str> = frame_lines.collect();
            let first = chunks.len();
            self.flatten_written(
                &element.children,
                chunks,
                path,
                base_metadata,
                &trail,
                &ctx,
                source,
            );
            // The container's own trivia (a comment above `services:`) is not
            // in any member: it rides with the first piece.
            let lead = element.leading_trivia().trim();
            if let (false, Some(head)) = (lead.is_empty(), chunks.get_mut(first)) {
                head.content = format!("{lead}\n{}", head.content);
            }
            if !rest.is_empty() {
                chunks.push(Candidate {
                    content: rest.join("\n"),
                    element_type: element.element_type.to_string(),
                    trail: trail.clone(),
                    parent_trail: trail.clone(),
                    line_start: element.line_end,
                    line_end: element.line_end,
                    metadata,
                    context: ctx,
                    unit_start: false,
                    break_before: false,
                });
            }
        }
    }

    /// The Markdown walk: every block is read under its heading path.
    ///
    /// `vecq` emits Markdown blocks as flat siblings, so the section structure
    /// is rebuilt here from heading levels. Each block carries the heading
    /// lines above it as `context`, which `pack` writes into the chunk text: a
    /// chunk starts with the headings it is about, and a table is never
    /// embedded without knowing what it is a table of.
    ///
    /// A heading is therefore not a candidate of its own — it travels with
    /// whatever follows it, and cannot be left at the end of the PREVIOUS chunk.
    /// The exception is a heading with no body, which would otherwise vanish.
    ///
    /// Only blocks at `block_depth` 0 are candidates. A list already contains
    /// its items' bytes, an item its paragraphs, a blockquote its paragraphs;
    /// emitting both embedded the same text two or three times.
    ///
    /// **A heading is a hard boundary.** Every block's `parent_trail` is its
    /// heading path, and the first block under each heading `starts_unit`, so
    /// neither the packer nor the floor joins two sections. With an empty
    /// parent, packing ran by size alone across `##`, and one chunk held three
    /// fault lines labelled as the first (reported by Ecosystem Experts the
    /// same day `c09ebfa` shipped). A short section is a short chunk — it still
    /// carries its heading path in its text.
    ///
    /// **Text as written.** A paragraph's `content` from vecq is rendered
    /// prose: backticks, emphasis markers and link URLs removed. Every other
    /// block is already its source slice, so the paragraph is taken from its
    /// source span too — `failCgroupV1: false` must embed as the code it is.
    fn flatten_markdown(
        &self,
        elements: &[DocumentElement],
        chunks: &mut Vec<Candidate>,
        path: &Path,
        base_metadata: &serde_json::Value,
        source: &str,
    ) {
        let attr_u64 = |e: &DocumentElement, key: &str| {
            e.attributes.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
        };
        let is_header = |e: &DocumentElement| e.element_type.to_string() == "header";
        // The block as it appears in the file. Trailing newlines are the
        // block's terminator, not its text; leaving them in put three blank
        // lines between packed members.
        let as_written = |e: &DocumentElement| -> String {
            let span = e.source_text(source);
            span.unwrap_or(&e.content)
                .trim_end()
                .trim_start_matches('\n')
                .to_string()
        };

        // Depth-0 blocks, plus the indices of blocks that follow a thematic
        // break (`---`): a break OPPORTUNITY before them (see
        // `Candidate::break_before`). The rule itself carries no words and is
        // not embedded; it only says where a cut would be best.
        let mut blocks: Vec<&DocumentElement> = Vec::new();
        let mut break_before: std::collections::HashSet<usize> = Default::default();
        let mut after_rule = false;
        for e in elements {
            if attr_u64(e, vecq::types::attr::BLOCK_DEPTH) != 0 {
                continue;
            }
            if e.element_type == vecq::types::ElementType::HorizontalRule {
                after_rule = true;
                continue;
            }
            if is_inline_span(&e.element_type.to_string()) || e.content.trim().is_empty() {
                continue;
            }
            if std::mem::take(&mut after_rule) {
                break_before.insert(blocks.len());
            }
            blocks.push(e);
        }

        // (level, heading line as written, heading text)
        let mut headings: Vec<(u64, String, String)> = Vec::new();
        // Heading path of the previous candidate: a change starts a unit.
        let mut previous_section: Option<String> = None;
        let lines = |h: &[(u64, String, String)]| -> Vec<String> {
            h.iter().map(|(_, line, _)| line.clone()).collect()
        };
        let names = |h: &[(u64, String, String)]| -> String {
            h.iter()
                .map(|(_, _, name)| name.as_str())
                .collect::<Vec<_>>()
                .join("::")
        };

        for (i, element) in blocks.iter().enumerate() {
            if is_header(element) {
                let level = attr_u64(element, "level");
                while headings.last().is_some_and(|(l, _, _)| *l >= level) {
                    headings.pop();
                }
                let line = element.content.trim().to_string();
                let name = element.name.clone().unwrap_or_default();
                let has_body = blocks
                    .get(i + 1)
                    .is_some_and(|next| !is_header(next) || attr_u64(next, "level") > level);
                if !has_body {
                    let context = lines(&headings);
                    let mut trail_h = headings.clone();
                    trail_h.push((level, line.clone(), name.clone()));
                    let trail = names(&trail_h);
                    let unit_start = previous_section.as_deref() != Some(trail.as_str());
                    previous_section = Some(trail.clone());
                    chunks.push(Candidate {
                        content: line.clone(),
                        element_type: element.element_type.to_string(),
                        metadata: self.element_metadata(element, path, base_metadata, &trail),
                        parent_trail: trail.clone(),
                        trail,
                        line_start: element.line_start,
                        line_end: element.line_end,
                        context,
                        unit_start,
                        break_before: break_before.contains(&i),
                    });
                }
                headings.push((level, line, name));
                continue;
            }

            let section = names(&headings);
            let trail = if headings.is_empty() {
                child_trail("", element)
            } else {
                section.clone()
            };
            let unit_start = previous_section.as_deref() != Some(section.as_str());
            previous_section = Some(section.clone());
            chunks.push(Candidate {
                content: as_written(element),
                element_type: element.element_type.to_string(),
                metadata: self.element_metadata(element, path, base_metadata, &trail),
                trail,
                parent_trail: section,
                line_start: element.line_start,
                line_end: element.line_end,
                context: lines(&headings),
                unit_start,
                break_before: break_before.contains(&i),
            });
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
            // Each member's context lines are written where they first apply:
            // all of them at the top of the chunk, then only the lines that
            // change. A run crossing from `## A` into `## B` therefore reads
            // as the document does, and never repeats `# Doc` per paragraph.
            let mut parts: Vec<&str> = Vec::new();
            let mut emitted: &[String] = &[];
            for c in run.iter() {
                let shared = emitted
                    .iter()
                    .zip(&c.context)
                    .take_while(|(a, b)| a == b)
                    .count();
                parts.extend(c.context[shared..].iter().map(String::as_str));
                parts.push(c.content.as_str());
                emitted = &c.context;
            }
            let content = parts.join("\n\n");

            // Metadata comes from the first member, which is the outermost /
            // earliest element in the run and therefore the one a reader would
            // name the chunk by. The span covers the whole run.
            let first = &run[0];
            let mut metadata = first.metadata.clone();
            if !first.context.is_empty() {
                // `content` begins with exactly this, so a reader can tell the
                // label from the source text and the splitter can repeat it.
                metadata.insert(
                    CONTEXT_LABEL_KEY.to_string(),
                    serde_json::Value::String(first.context.join("\n\n")),
                );
            }
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

        // A file smaller than the target is ONE chunk — when it has at most one
        // unit (see below; it was once unconditional).
        //
        // Splitting it cannot improve retrieval — every piece would be returned
        // for the same queries — and it costs the reader the surrounding
        // context. Size was only ever a proxy for the real question; below the
        // target there is no question to ask. Without this, a 600-byte file
        // with two top-level sections still came out as two chunks, because the
        // run breaks on a parent-trail change.
        //
        // **Only for a file with at most one unit** (a Markdown section, a
        // named code unit — `Candidate::starts_unit`). Size caps a unit; it
        // never merges units. Applied unconditionally, this returned a
        // 2-function `.py`/`.rs` as one chunk named for the first function
        // (named units never merge) and a small multi-section Markdown file
        // as one chunk named for its first section, before any unit boundary
        // was consulted (Ecosystem Experts on `b79f78e`). The argument above
        // holds only where there is nothing to keep apart: flat prose, a small
        // JSON document, a file with one function.
        let whole_file: usize = candidates.iter().map(|c| density(&c.content)).sum();
        let units = candidates.iter().filter(|c| c.starts_unit()).count();
        if whole_file <= self.config.pack_target_bytes && !candidates.is_empty() && units <= 1 {
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

            // Named-unit break: a function/method/class/struct/interface/trait
            // starts a new run regardless of trail, so two adjacent named units
            // never merge into one chunk. A Markdown section start is a unit too
            // — see `flatten_markdown`.
            let is_named = cand.starts_unit();
            let breaks_sibling_run = is_named
                || run
                    .first()
                    .map(|f| f.parent_trail != cand.parent_trail)
                    .unwrap_or(false);
            let mut would_overflow = run_len + cand_len > self.config.pack_target_bytes;
            let stands_alone = cand_len >= self.config.pack_target_bytes;

            // Break opportunity (`Candidate::break_before`): about to overflow,
            // cut at the LATEST opportunity in the run rather than here, so the
            // content after a `---` starts the next chunk. If the remainder then
            // fits with `cand`, the run continues from it.
            if would_overflow && !breaks_sibling_run && !stands_alone {
                if let Some(k) = run.iter().rposition(|c| c.break_before).filter(|&k| k > 0) {
                    let tail = run.split_off(k);
                    runs.push((std::mem::take(&mut run), false));
                    run = tail;
                    let tail_len: usize = run.iter().map(|c| density(&c.content)).sum();
                    would_overflow = tail_len + cand_len > self.config.pack_target_bytes;
                }
            }

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
                // Don't fold a run that starts with a named unit into a
                // previous run. A named unit must not be merged with a
                // different named unit, even if it's below the floor. Same
                // for a Markdown section start.
                let starts_with_named = run.first().map(|c| c.starts_unit()).unwrap_or(false);
                if !starts_with_named {
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
                // Don't fold a named-unit fragment backward.
                let frag_starts_with_named = frag.first().map(|c| c.starts_unit()).unwrap_or(false);
                if !frag_starts_with_named {
                    match out.last_mut() {
                        Some((prev, prev_solo)) if !*prev_solo => prev.extend(frag),
                        // Its only neighbour stands alone; emit it rather than
                        // corrupt that element. See the doc comment.
                        _ => out.push((frag, false)),
                    }
                } else {
                    out.push((frag, false));
                }
            }
        }

        out
    }
}

/// A container's own text with its children's text removed: what remains is
/// the container's frame (header, closing line, members the walker did not
/// emit as children). Children are located in order, each searched from where
/// the previous one ended, so repeated text cannot match the wrong child. A
/// child that cannot be located is left in place — over-including is safe,
/// cutting the wrong text is not. Blank lines left by the cuts are collapsed.
fn container_frame(content: &str, children: &[DocumentElement]) -> String {
    let mut out = String::new();
    let mut at = 0;
    for child in children {
        let piece = child.content.trim();
        if piece.is_empty() {
            continue;
        }
        if let Some(found) = content.get(at..).and_then(|rest| rest.find(piece)) {
            out.push_str(&content[at..at + found]);
            at += found + piece.len();
        }
    }
    out.push_str(&content[at..]);
    let mut collapsed = String::new();
    let mut blank = false;
    for line in out.lines() {
        if line.trim().is_empty() {
            if !blank && !collapsed.is_empty() {
                collapsed.push('\n');
            }
            blank = true;
        } else {
            collapsed.push_str(line);
            collapsed.push('\n');
            blank = false;
        }
    }
    collapsed.trim_end().to_string()
}

/// `parent::name`, or the element type for an unnamed element.
fn child_trail(parent_trail: &str, element: &DocumentElement) -> String {
    let own = element
        .name
        .clone()
        .unwrap_or_else(|| element.element_type.to_string());
    if parent_trail.is_empty() {
        own
    } else {
        format!("{parent_trail}::{own}")
    }
}

/// Bytes `pack` will spend writing these context lines, separators included.
fn context_len(context: &[String]) -> usize {
    context.iter().map(|l| l.len() + 2).sum()
}

/// `vecq` names array items `[N]`; anything else is an object member.
fn is_json_array_item(element: &DocumentElement) -> bool {
    element
        .name
        .as_deref()
        .is_some_and(|n| n.starts_with('[') && n.ends_with(']'))
}

/// A JSON value as embeddable text: `"key": value` for an object member, the
/// bare value for an array item.
///
/// `vecq`'s leaf content is the bare value, and a bare `"1.25"` says nothing
/// about what is 1.25. YAML and TOML already render `key: value` for the same
/// reason; this brings JSON into line.
///
/// Containers too: a nested object or array emitted whole was its bare value,
/// so `"numbers": [0, -1]` embedded as `[0,-1]` and `"level13": {..}` as
/// `{"level12": ..}` — the key lost. Found by the T1.19 JSON coverage gate.
///
/// **As written**, when vecq supplies the source span: `content` is a
/// re-serialisation (keys sorted, comments gone, `1e21` → `1e+21`), so the
/// text is sliced from the source instead — the rule Markdown paragraphs
/// follow too. The member's comments (`leading_comment` / `trailing_comment`)
/// come with it: in JSONC they are usually the only place a setting's reason
/// is written.
fn render_json_member(element: &DocumentElement, source: &str) -> String {
    let (leading, trailing) = json_comments(element);
    let mut out = String::new();
    if let Some(c) = leading {
        out.push_str(&c);
        out.push('\n');
    }
    out.push_str(&json_as_written(element, source));
    if let Some(c) = trailing {
        out.push(' ');
        out.push_str(&c);
    }
    out
}

/// The member as written (no comments), or rendered when no span is known.
fn json_as_written(element: &DocumentElement, source: &str) -> String {
    if let Some(text) = element.source_text(source) {
        return text.to_string();
    }
    match &element.name {
        Some(key) if !is_json_array_item(element) => {
            let key = serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""));
            format!("{key}: {}", element.content)
        }
        _ => element.content.clone(),
    }
}

fn json_comments(element: &DocumentElement) -> (Option<String>, Option<String>) {
    (
        owned_comment_text(element.leading_trivia()).map(|(c, _)| c),
        owned_comment_text(element.trailing_trivia()).map(|(c, _)| c),
    )
}

/// The comment text in an element's owned trivia (vecq `SourceInfo`), and how
/// many lines it spans from its first comment line to the end of the trivia.
///
/// Besides an element's comments it holds the whitespace, line breaks and
/// separators (`,` `;`) around it, and a
/// flattened JSON root's first and last members hold its brackets. None of
/// that is content — only lines that carry words are kept, each trimmed of its
/// indentation and of the separators BEFORE its comment (`, // why` → `// why`).
/// Never after: a comment runs to the end of its line, so a trailing `,` or `;`
/// is the comment's own prose (`// exports, requires,`) — trimming both ends
/// dropped it (2026-314, measured on JavaScript). `None` when the trivia holds
/// no comment.
fn owned_comment_text(trivia: &str) -> Option<(String, usize)> {
    let lines: Vec<&str> = trivia.split('\n').collect();
    let first = lines.iter().position(|l| vecdb_common::has_meaning(l))?;
    let kept: Vec<&str> = lines[first..]
        .iter()
        .filter(|l| vecdb_common::has_meaning(l))
        .map(|l| l.trim().trim_start_matches([',', ';']).trim())
        .collect();
    Some((kept.join("\n"), lines.len() - 1 - first))
}

/// `content` with the comments this element owns (vecq `SourceInfo` trivia):
/// the comment above it on its own lines, and a same-line comment after it.
/// Ownership is decided once, in vecq (`parsers::ownership`); a comment is
/// therefore embedded with the code it describes, never at the end of the
/// previous chunk. Returns the text and how many lines the leading comment
/// adds above `line_start`. Whitespace-only trivia adds nothing.
/// `content` with the trivia it owns kept AS WRITTEN — every non-blank line,
/// not only comments: for YAML and HTML the separators are structure (`---`
/// between documents) and a comment is markup (`<!-- -->`). Blank lines at
/// either end are dropped; lines are not re-indented. Returns the text and
/// how many lines the leading trivia adds above `line_start`.
fn with_trivia_as_written(element: &DocumentElement, content: &str) -> (String, usize) {
    let lead = element.leading_trivia();
    let lead_kept = lead.trim_matches(|c: char| c == '\n' || c == '\r');
    let lead_kept = if lead_kept.trim().is_empty() {
        ""
    } else {
        lead_kept.trim_start()
    };
    let mut out = String::new();
    let mut lines_above = 0;
    if !lead_kept.is_empty() {
        // Lines from the first kept trivia line down to the element's own line.
        let from = lead.find(lead_kept).unwrap_or(0);
        lines_above = lead[from..].matches('\n').count();
        out.push_str(lead_kept.trim_end());
        out.push('\n');
    }
    out.push_str(content);
    let trail = element.trailing_trivia();
    let trail_kept = trail.trim();
    if !trail_kept.is_empty() {
        // Same line as the element (a same-line comment) or a line of its own.
        let same_line = !trail[..trail.find(trail_kept).unwrap_or(0)].contains('\n');
        out.push(if same_line { ' ' } else { '\n' });
        out.push_str(trail_kept);
    }
    (out, lines_above)
}

fn with_owned_comments(element: &DocumentElement, content: &str) -> (String, usize) {
    let mut out = String::new();
    let mut lines_above = 0;
    if let Some((lead, above)) = owned_comment_text(element.leading_trivia()) {
        lines_above = above;
        out.push_str(&lead);
        out.push('\n');
    }
    out.push_str(content);
    if let Some((trail, _)) = owned_comment_text(element.trailing_trivia()) {
        out.push(' ');
        out.push_str(&trail);
    }
    (out, lines_above)
}

/// Longest single field that may identify a record, rendered.
const JSON_IDENTITY_FIELD_MAX: usize = 96;
/// Most bytes of identifying fields repeated into each fragment's label.
const JSON_IDENTITY_BUDGET: usize = 256;

/// Indices of the fields that identify a record whose pieces are embedded
/// separately: its SHORT scalar members (`id`, `name`, `version`), in order,
/// within a byte budget.
///
/// Length is the test, deliberately, rather than a list of likely key names —
/// a short scalar is a label and a long one is content, whatever the schema
/// calls it. The budget keeps a 1000-key object from turning its label into a
/// second copy of itself.
///
/// A member with a comment is never identity: identity lives only in labels,
/// and a comment is content, not a label.
fn json_identity(siblings: &[DocumentElement], source: &str) -> Vec<usize> {
    let mut used = 0;
    let mut picked = Vec::new();
    for (i, el) in siblings.iter().enumerate() {
        // Containers — `{}` and `[]` included — are `Block`, scalars
        // `Variable` (vecq json.rs). Not `content`'s first byte: a member's
        // content is `"key": value` as written, so `starts_with('{')` was
        // false for every member, an empty `{}`/`[]` passed as a scalar and
        // rode in the label, where the T1.19 gate credits no empties (2026-11-10).
        let scalar = el.children.is_empty()
            && el.element_type == vecq::types::ElementType::Variable
            && !is_json_array_item(el)
            && json_comments(el) == (None, None);
        if !scalar {
            continue;
        }
        let len = json_as_written(el, source).len() + 1;
        if len <= JSON_IDENTITY_FIELD_MAX && used + len <= JSON_IDENTITY_BUDGET {
            used += len;
            picked.push(i);
        }
    }
    picked
}

/// How many top-level values a JSON/JSONL text holds (counting stops at 2).
///
/// `vecq` flattens every root's members into one sibling list, so for JSONL
/// the top level is several records interleaved. One record's fields must
/// never label another's, so identity is only taken from a single root. A
/// text strict JSON cannot read (comments, trailing commas) is parsed by vecq
/// as JSON5, which yields one root.
fn json_root_count(content: &str) -> usize {
    serde_json::Deserializer::from_str(content)
        .into_iter::<serde::de::IgnoredAny>()
        .take(2)
        .take_while(Result::is_ok)
        .count()
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
        //
        // JSON and Markdown each need their own walk, because what an element's
        // `content` holds differs: a JSON object's content already contains
        // its children; Markdown blocks arrive flat, with sections implied by
        // heading levels. The generic walk is right for source code.
        let elements = &parsed_doc.elements;
        match self.file_type {
            FileType::Json => {
                let single_root = json_root_count(content) <= 1;
                self.flatten_json(
                    elements,
                    &mut candidates,
                    path,
                    &base_meta,
                    "",
                    &[],
                    single_root,
                    content,
                )
            }
            FileType::Markdown => {
                self.flatten_markdown(elements, &mut candidates, path, &base_meta, content)
            }
            FileType::Yaml | FileType::Html => self.flatten_written(
                elements,
                &mut candidates,
                path,
                &base_meta,
                "",
                &[],
                content,
            ),
            _ => self.flatten_elements(elements, &mut candidates, path, &base_meta, "", content),
        }

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
        sizing: Option<crate::parsers::ParserSizing>,
    ) -> Option<Box<dyn Parser>> {
        file_type.is_supported().then(|| {
            let mut cfg = self.adapter_config();
            if let Some(s) = sizing {
                cfg.pack_target_bytes = s.pack_target_bytes;
                cfg.max_chunk_bytes = s.max_chunk_bytes;
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

    /// REGRESSION (2026-314): separators are trimmed only before a comment
    /// (`, // why`), never from its end. `// exports, requires,` lost its
    /// trailing comma before this — a 0.6% token loss measured on a
    /// JavaScript module by the chunk coverage check.
    #[test]
    fn owned_comment_keeps_its_own_trailing_punctuation() {
        let (text, above) =
            owned_comment_text("\n// exports, requires,\n// and top-level work;\n").unwrap();
        assert_eq!(text, "// exports, requires,\n// and top-level work;");
        assert_eq!(above, 2);
        let (sep, _) = owned_comment_text(", // why\n").unwrap();
        assert_eq!(
            sep, "// why",
            "the separator before a comment is not content"
        );
    }

    /// REGRESSION (2026-314): a statement vecq reports once per imported name
    /// is embedded once, not once per name.
    #[tokio::test]
    async fn a_statement_shared_by_several_imports_is_embedded_once() {
        let src = "from typing import List, Dict\n\n\ndef f():\n    return 1\n";
        let chunks = VecqParserAdapter::new(FileType::Python)
            .parse(src, Path::new("imports.py"), None)
            .await
            .unwrap();
        let all: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert_eq!(
            all.matches("from typing import List, Dict").count(),
            1,
            "{all}"
        );
    }

    async fn ids_for(src: &str) -> Vec<String> {
        let adapter = VecqParserAdapter::new(FileType::Rust);
        let chunks = adapter
            .parse(src, Path::new("probe.rs"), None)
            .await
            .expect("vecq must parse trivial Rust");
        assert!(!chunks.is_empty(), "fixture produced no chunks");
        chunks.into_iter().map(|c| c.id).collect()
    }

    /// REGRESSION: Shifting every element down by two blank lines changes
    /// `line_start` for every chunk but changes no content. Chunk IDs MUST
    /// be unaffected.
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
             regressed to incorporating line numbers"
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
            "",
        );
        chunks
    }

    use vecq::types::{DocumentElement as DocEl, ElementType};

    // ── Packing ────────────────────────────────────────
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
                pack_target_bytes: 2048,
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
    /// collapses it to one chunk and the floor is never exercised. (Since the
    /// rule became unit-aware, headings already keep these apart; the sizing
    /// still guards a future unit-less variant.)
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

    /// A file below the target is ONE chunk when it has no units to keep
    /// apart. Splitting it cannot improve retrieval — every piece answers the
    /// same queries — and it costs the reader the surrounding context.
    #[test]
    fn file_under_target_without_units_is_a_single_chunk() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 2048,
            ..VecqAdapterConfig::default()
        };
        let chunks = pack_with(cfg, siblings(3, 50));
        assert_eq!(chunks.len(), 1, "a small unit-less file is one chunk");
    }

    /// A file below the target with two sections is TWO chunks — size caps a
    /// unit, it never merges units.
    ///
    /// This test once asserted the opposite ("A file below the target is ONE
    /// chunk, even when its elements sit under different parents"), the old
    /// whole-file rule. It was superseded by the named-unit rule and
    /// "sections never merge"; Ecosystem Experts found the rule still applied
    /// on `b79f78e`.
    #[tokio::test]
    async fn file_under_target_keeps_its_units_apart() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 2048,
            max_chunk_bytes: 7200,
        };
        let md = "# One\n\nfirst section body\n\n# Two\n\nsecond section body\n";
        let chunks = embedded(FileType::Markdown, "two.md", md, sizing).await;
        assert_eq!(
            chunks.len(),
            2,
            "two sections of a small file must not share a chunk"
        );
        assert!(chunks[1].content.contains("second section body"));
        assert!(!chunks[1].content.contains("first section body"));

        // Same for named code units: two functions, two chunks, each named
        // for its own function (not the first one's).
        let rs = "fn alpha() -> u32 {\n    1\n}\n\nfn beta() -> u32 {\n    2\n}\n";
        let chunks = parsed(FileType::Rust, "two.rs", rs, sizing).await;
        assert_eq!(chunks.len(), 2, "two functions must not share a chunk");
        let named = |c: &Chunk| {
            c.metadata
                .get("crumbtrail")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        assert_eq!(named(&chunks[0]), "alpha");
        assert_eq!(named(&chunks[1]), "beta");
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

    // ── JSON descent, labels, duplication ─────────────────────────────────
    //
    // The first tests built a synthetic tree and asserted on the ratio branch,
    // so they passed over the report: one asserted the inverted rule, the other
    // checked for an element NAME that is never in any content and could not
    // fail. These go through the real vecq parse with the reported fixtures,
    // and assert on chunk TEXT.

    /// A source-code wrapper fully covered by its children is dropped at ANY
    /// size: keeping it embeds its body twice.
    ///
    /// The fix gated the drop on `children_len <=
    /// pack_target_bytes`, so a large wrapper was kept beside its children.
    #[test]
    fn redundant_code_wrapper_dropped_at_any_size() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 100,
            ..VecqAdapterConfig::default()
        };

        let child_a = DocEl::new(
            ElementType::Function,
            Some("a".into()),
            "x".repeat(600),
            2,
            2,
        );
        let child_b = DocEl::new(
            ElementType::Function,
            Some("b".into()),
            "y".repeat(600),
            3,
            3,
        );
        let parent = DocEl::new(
            ElementType::Block,
            Some("wrapper".to_string()),
            format!("{}{}", child_a.content, child_b.content),
            1,
            4,
        )
        .with_children(vec![child_a, child_b]);

        let chunks = pack_with(cfg, vec![parent]);
        let total: usize = chunks.iter().map(|c| c.content.matches('x').count()).sum();
        assert_eq!(total, 600, "the wrapper's bytes were embedded twice");
        assert!(
            !chunks
                .iter()
                .any(|c| c.metadata.get("crumbtrail") == Some(&serde_json::json!("wrapper"))),
            "a fully-covered wrapper became a chunk of its own"
        );
    }

    /// The `ee` profile's sizing, which is where the regression was measured.
    const EE: crate::parsers::ParserSizing = crate::parsers::ParserSizing {
        pack_target_bytes: 2048,
        max_chunk_bytes: 7200,
    };

    async fn parsed(
        ft: FileType,
        name: &str,
        src: &str,
        sizing: crate::parsers::ParserSizing,
    ) -> Vec<Chunk> {
        VecqParserFactory::default()
            .get_parser_sized(ft, Some(sizing))
            .expect("vecq parses this type")
            .parse(src, Path::new(name), None)
            .await
            .expect("fixture parses")
    }

    /// What reaches the embedder: parsed chunks, with oversized ones split by
    /// the pipeline exactly as `flush_chunks` does.
    async fn embedded(
        ft: FileType,
        name: &str,
        src: &str,
        sizing: crate::parsers::ParserSizing,
    ) -> Vec<Chunk> {
        let mut out = Vec::new();
        for c in parsed(ft, name, src, sizing).await {
            if c.content.len() > sizing.max_chunk_bytes {
                out.extend(
                    crate::ingestion::pipeline::split_oversized(&c, sizing.max_chunk_bytes)
                        .await
                        .unwrap(),
                );
            } else {
                out.push(c);
            }
        }
        out
    }

    const SENTENCE: &str = "In Kubernetes 1.22, the networking.k8s.io/v1beta1 Ingress API stopped \
        being served; manifests that still name it are rejected by the API server, and the fix \
        is to move to the stable group. ";

    /// REGRESSION: the §7 inline fixture.
    ///
    /// Record 1 is ~1.2 KB with a children/parent ratio of 0.97 and children
    /// under `pack_target_bytes`: the exact shape the inverted fix destroyed,
    /// leaving a bare `detail` string with crumbtrail `[1]::detail`. Record 0
    /// came back once whole and again as three loose values.
    ///
    /// Every record must come back as ONE parseable object holding its `name`,
    /// and no piece of any chunk may be a bare value.
    #[tokio::test]
    async fn json_records_come_back_whole_and_exactly_once() {
        let src = serde_json::json!([
            {"name": "PSP removal", "version": "1.25", "detail": "PodSecurityPolicy removed."},
            {"name": "Ingress v1beta1", "version": "1.22", "detail": SENTENCE.repeat(6)},
        ])
        .to_string();

        let chunks = embedded(FileType::Json, "records.json", &src, EE).await;
        let mut names = Vec::new();
        for c in &chunks {
            for piece in c.content.split("\n\n") {
                let obj: serde_json::Map<String, serde_json::Value> = serde_json::from_str(piece)
                    .unwrap_or_else(|_| panic!("chunk piece is not a JSON object: {piece:.80}"));
                names.push(obj["name"].as_str().unwrap().to_string());
            }
        }
        names.sort();
        assert_eq!(
            names,
            ["Ingress v1beta1", "PSP removal"],
            "each record must be embedded whole, exactly once"
        );
    }

    /// REGRESSION: `oversize.json`.
    ///
    /// One value far over the ceiling must be split, and EVERY part must say,
    /// in its text, which key it is a piece of and which record it belongs to.
    /// Before, only `content` was embedded and no part carried `title` or
    /// `WAY_TOO_LONG`; the labels lived in payload the vector never sees.
    #[tokio::test]
    async fn oversized_json_value_parts_carry_key_and_identity() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 2048,
            max_chunk_bytes: 1000,
        };
        let big = SENTENCE.repeat(25);
        let src = serde_json::json!({"id": "WAY_TOO_LONG", "title": big}).to_string();

        let parts = embedded(FileType::Json, "oversize.json", &src, sizing).await;
        assert!(parts.len() >= 4, "fixture must split into several parts");
        for p in &parts {
            assert!(p.content.len() <= 1000, "part over the ceiling");
            assert!(
                p.content.contains("WAY_TOO_LONG") && p.content.contains("title"),
                "part {:?} does not say what it is: {:.120}",
                p.metadata.get("split_part"),
                p.content
            );
        }
        // Nothing lost: the bodies, label removed, rebuild the member exactly
        // as the source wrote it — key included (operator default, 2026-300:
        // an oversized value is embedded as written, like every other member).
        let label = parts[0].metadata[CONTEXT_LABEL_KEY]
            .as_str()
            .unwrap()
            .to_string();
        let body: String = parts
            .iter()
            .map(|p| {
                p.content
                    .strip_prefix(&label)
                    .unwrap()
                    .strip_prefix("\n\n")
                    .unwrap()
            })
            .collect();
        assert_eq!(
            body,
            format!("\"title\":{}", serde_json::to_string(&big).unwrap())
        );
    }

    /// REGRESSION: `oversize.md`.
    ///
    /// The heading used to close the PREVIOUS chunk, and the section's body was
    /// embedded as anonymous `paragraph` parts. Every chunk holding the huge
    /// section's text must now open with its heading path, and the crumbtrail
    /// must be that path rather than an element type.
    #[tokio::test]
    async fn markdown_heading_path_prefixes_every_part() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 2048,
            max_chunk_bytes: 1000,
        };
        let src = format!(
            "# Oversize section test\n\n## Before\n\n{}\n\n## The huge section\n\n{}\n\n## After\n\n{}\n",
            SENTENCE.repeat(2).replace("1.22", "1.20"),
            SENTENCE.repeat(20).replace("1.22", "1.25"),
            SENTENCE.repeat(2).replace("1.22", "1.29"),
        );

        let chunks = embedded(FileType::Markdown, "oversize.md", &src, sizing).await;
        let huge: Vec<_> = chunks
            .iter()
            .filter(|c| c.content.contains("1.25"))
            .collect();
        assert!(huge.len() >= 3, "fixture must split the huge section");
        for c in &huge {
            assert!(
                c.content
                    .starts_with("# Oversize section test\n\n## The huge section\n\n"),
                "part does not open with its heading path: {:.80}",
                c.content
            );
            assert_eq!(
                c.metadata["crumbtrail"],
                serde_json::json!("Oversize section test::The huge section")
            );
        }
        for c in &chunks {
            let last = c.content.trim_end().rsplit("\n\n").next().unwrap();
            assert!(
                !last.starts_with('#'),
                "a chunk ends on a heading: {last:?}"
            );
        }
    }

    /// REGRESSION (Ecosystem Experts on `c09ebfa`): two Markdown sections never
    /// share a chunk, even when both are small and the floor would fold them.
    ///
    /// Shaped like `fault-lines.md`, where one chunk held Fault Lines C2, S and
    /// K under C2's label. The sections are each well under the pack target,
    /// and one is under `min_chunk_bytes`, so only the heading boundary keeps
    /// them apart. Padding keeps the file over the target: below it the whole
    /// file was one chunk by design until `b79f78e` (now only a unit-less file
    /// is — see `file_under_target_keeps_its_units_apart`).
    /// `n` words tagged `tag`, so a test can find where each block landed.
    fn words(tag: &str, n: usize) -> String {
        (0..n)
            .map(|i| format!("{tag}{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// REGRESSION (operator's example, 2026-300): a thematic break is a break
    /// OPPORTUNITY. A block that nearly fills the chunk, `---`, a block that
    /// would still fit, then one that overflows: the cut lands at the `---`,
    /// so what follows the separator starts the next chunk together — and
    /// both chunks are labelled with the section's heading.
    #[tokio::test]
    async fn markdown_rule_is_where_an_overflowing_run_is_cut() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 600,
            max_chunk_bytes: 7200,
        };
        let src = format!(
            "### Example\n\n{}\n\n---\n\n{}\n\n{}\n",
            words("big", 80),  // ~390 non-ws: most of the target
            words("fits", 20), // ~110: still fits beside it (~500)
            words("over", 30), // ~170: now the run overflows (~670)
        );
        let chunks = embedded(FileType::Markdown, "rule.md", &src, sizing).await;
        let holding = |tag: &str| {
            chunks
                .iter()
                .position(|c| c.content.contains(&format!("{tag}0")))
                .unwrap()
        };
        assert_ne!(
            holding("big"),
            holding("fits"),
            "the cut must land at the `---`"
        );
        assert_eq!(
            holding("fits"),
            holding("over"),
            "content after the separator starts the next chunk together"
        );
        for c in &chunks {
            assert!(
                c.content.starts_with("### Example"),
                "unlabelled chunk: {}",
                c.content
            );
        }
    }

    /// The other half of a break OPPORTUNITY: when the run fits, the `---`
    /// changes nothing — both sides share the chunk.
    #[tokio::test]
    async fn markdown_rule_does_not_split_what_fits() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 600,
            max_chunk_bytes: 7200,
        };
        // A large other section keeps the file over the target, so packing runs.
        let src = format!(
            "## Other\n\n{}\n\n### Example\n\n{}\n\n---\n\n{}\n",
            words("filler", 130),
            words("left", 15),
            words("right", 15),
        );
        let chunks = embedded(FileType::Markdown, "fits.md", &src, sizing).await;
        let holding = |tag: &str| {
            chunks
                .iter()
                .position(|c| c.content.contains(&format!("{tag}0")))
                .unwrap()
        };
        assert_eq!(
            holding("left"),
            holding("right"),
            "a fitting run is never cut at `---`"
        );
    }

    #[tokio::test]
    async fn markdown_sections_never_share_a_chunk() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 1200,
            max_chunk_bytes: 7200,
        };
        let src = format!(
            "# Fault Lines\n\n## Node\n\n### C2: cgroup v1\n\n- `failCgroupV1` default `true` at 1.35.\n\n\
             ### S: SPDY\n\n- exec moves to WebSockets.\n\n### K: kubeadm\n\n{}\n",
            SENTENCE.repeat(10)
        );
        let chunks = embedded(FileType::Markdown, "fault-lines.md", &src, sizing).await;
        for (marker, own) in [
            ("failCgroupV1", "C2"),
            ("WebSockets", "S: SPDY"),
            ("networking.k8s.io", "K: kubeadm"),
        ] {
            let holding: Vec<_> = chunks
                .iter()
                .filter(|c| c.content.contains(marker))
                .collect();
            assert_eq!(holding.len(), 1, "{marker} in {} chunks", holding.len());
            let c = holding[0];
            let others = ["C2", "S: SPDY", "K: kubeadm"]
                .into_iter()
                .filter(|h| *h != own);
            for other in others {
                assert!(
                    !c.content.contains(&format!("### {other}")),
                    "section {own} shares a chunk with {other}:\n{}",
                    c.content
                );
            }
            let trail = c
                .metadata
                .get("crumbtrail")
                .and_then(|v| v.as_str())
                .unwrap();
            assert!(trail.contains(own), "{marker} labelled {trail:?}");
        }
    }

    /// REGRESSION (Ecosystem Experts on `c09ebfa`): a paragraph embeds as
    /// written. vecq's paragraph `content` is rendered text, so backticks,
    /// emphasis and link URLs were gone before the model saw them.
    #[tokio::test]
    async fn markdown_paragraph_embeds_as_written() {
        let src = "# Platform\n\nEscape hatch: `failCgroupV1: false` (**not** recommended, see [KEP-5573](https://k8s.io/kep)).\n";
        let all: String = embedded(FileType::Markdown, "p.md", src, EE)
            .await
            .iter()
            .map(|c| c.content.clone())
            .collect();
        assert!(
            all.contains(
                "`failCgroupV1: false` (**not** recommended, see [KEP-5573](https://k8s.io/kep))."
            ),
            "{all}"
        );
    }

    /// REGRESSION: nested Markdown blocks are embedded once.
    ///
    /// vecq emits blocks flat — a list, each of its items, and each paragraph
    /// inside a loose item — and every one was a candidate, so list text was
    /// embedded two or three times. A nested list also truncated its parent.
    #[tokio::test]
    async fn markdown_nested_blocks_are_embedded_once() {
        let src = "# Lists\n\n- tight one\n- tight two\n  - nested\n\nloose:\n\n- para one\n\n- para two\n\n> quoted para\n";
        let chunks = embedded(FileType::Markdown, "lists.md", src, EE).await;
        let all: String = chunks.iter().map(|c| c.content.as_str()).collect();
        for needle in [
            "tight one",
            "tight two",
            "nested",
            "para one",
            "para two",
            "quoted para",
        ] {
            assert_eq!(
                all.matches(needle).count(),
                1,
                "{needle:?} embedded more than once"
            );
        }
    }

    /// JSONL roots arrive from vecq as one interleaved sibling list. One
    /// record's identifying fields must never label another record's pieces.
    #[tokio::test]
    async fn jsonl_records_never_label_each_other() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 2048,
            max_chunk_bytes: 600,
        };
        let src = format!(
            "{}\n{}\n",
            serde_json::json!({"name": "rec-a", "detail": SENTENCE.repeat(6)}),
            serde_json::json!({"name": "rec-b", "detail": SENTENCE.repeat(6)}),
        );
        for c in embedded(FileType::Json, "records.jsonl", &src, sizing).await {
            assert!(
                !(c.content.contains("rec-a") && c.content.contains("rec-b")),
                "one chunk carries both records' identity: {:.120}",
                c.content
            );
        }
    }

    /// Every payload key the adapter or the splitter writes must be reserved,
    /// or user metadata under that key is silently overwritten. Measured on
    /// real chunks, so a new built-in key fails here.
    #[tokio::test]
    async fn reserved_keys_cover_every_key_the_adapter_writes() {
        let sizing = crate::parsers::ParserSizing {
            pack_target_bytes: 64,
            max_chunk_bytes: 400,
        };
        // `## C` holds two short blocks of one section: the packed run that
        // writes `packed_elements` (units never pack together).
        let md = format!(
            "# A\n\n{}\n\n## B\n\n- x\n- y\n\n## C\n\nshort one.\n\nshort two.\n",
            SENTENCE.repeat(5)
        );
        let json = serde_json::json!({"id": "x", "body": SENTENCE.repeat(5)}).to_string();
        let mut chunks = embedded(FileType::Markdown, "k.md", &md, sizing).await;
        chunks.extend(embedded(FileType::Json, "k.json", &json, sizing).await);
        chunks.extend(parsed(FileType::Rust, "k.rs", SRC, sizing).await);
        assert!(chunks.iter().any(|c| c.metadata.contains_key("split_part")));
        assert!(chunks
            .iter()
            .any(|c| c.metadata.contains_key("packed_elements")));

        for c in &chunks {
            for key in c.metadata.keys() {
                assert!(
                    crate::ingestion::options::is_reserved_payload_key(key),
                    "the adapter writes '{key}', which RESERVED_PAYLOAD_KEYS does not list"
                );
            }
        }
    }

    // ── Pack straddles sibling boundaries ──────────────────────────────────

    /// Two adjacent named units (functions) with the same parent trail must
    /// produce separate chunks, not one merged chunk.
    #[test]
    fn adjacent_named_units_do_not_merge() {
        // pack_target_bytes must be < the combined density of the two functions
        // (~38) so the whole-file optimization does not collapse them (no longer
        // needed since that rule is unit-aware, kept as a margin), and > each
        // individual function's density (~20) so neither stands alone.
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 30,
            ..VecqAdapterConfig::default()
        };

        let fn_a = DocEl::new(
            ElementType::Function,
            Some("alpha".to_string()),
            "pub fn alpha() -> u32 { 1 }".to_string(),
            1,
            1,
        );
        let fn_b = DocEl::new(
            ElementType::Function,
            Some("beta".to_string()),
            "pub fn beta() -> u32 { 2 }".to_string(),
            2,
            2,
        );

        let chunks = pack_with(cfg, vec![fn_a, fn_b]);

        assert_eq!(
            chunks.len(),
            2,
            "adjacent named units must not merge into one chunk"
        );
    }

    /// Unnamed elements under the same parent still pack together.
    #[test]
    fn unnamed_elements_still_pack() {
        let cfg = VecqAdapterConfig {
            pack_target_bytes: 2048,
            ..VecqAdapterConfig::default()
        };

        let para_a = DocEl::new(
            ElementType::Paragraph,
            None,
            "first paragraph".to_string(),
            1,
            1,
        );
        let para_b = DocEl::new(
            ElementType::Paragraph,
            None,
            "second paragraph".to_string(),
            2,
            2,
        );

        let chunks = pack_with(cfg, vec![para_a, para_b]);

        assert_eq!(
            chunks.len(),
            1,
            "unnamed elements under the same parent must pack together"
        );
    }
}
