//! Chunk-fit checking and the oversize policy.
//!
//! The invariant both policies preserve:
//!
//!   **Never store a chunk whose metadata claims more than its content contains.**
//!
//! Truncation breaks it — a chunk labelled `main.rs:1-400` holding 60% of that
//! range is a lie no reader can detect, and only a re-ingest repairs it.
//! Splitting does not: `split_part: 2` with real line bounds honestly describes
//! honest content. Skipping does not either: the content is simply absent, and
//! the run summary says so.
//!
//! Neither policy aborts the run. An oversized chunk is a configuration problem.

use vecdb_core::config::{check_chunk_fit, ChunkFit, IngestionConfig, OversizePolicy};

#[test]
fn oversize_policy_defaults_to_split() {
    assert_eq!(
        OversizePolicy::default(),
        OversizePolicy::Split,
        "content preserved and honestly labelled is the better default; skip is \
         opt-in for people who want the corpus exactly as precise as the source"
    );
    let parsed: IngestionConfig = toml::from_str("target_chunk_size = 512").unwrap();
    assert_eq!(
        parsed.on_oversize, None,
        "unset means 'use the default', not 'skip'"
    );
}

#[test]
fn oversize_policy_parses_from_config() {
    let skip: IngestionConfig = toml::from_str("on_oversize = \"skip\"").unwrap();
    assert_eq!(skip.on_oversize, Some(OversizePolicy::Skip));
    let split: IngestionConfig = toml::from_str("on_oversize = \"split\"").unwrap();
    assert_eq!(split.on_oversize, Some(OversizePolicy::Split));
}

/// A target at or above the window cannot work: the policy fires on every
/// full-size chunk, so it is a config error and not a runtime surprise.
#[test]
fn target_at_or_above_the_window_is_impossible() {
    assert_eq!(check_chunk_fit(6144, 512), ChunkFit::Impossible);
    assert_eq!(
        check_chunk_fit(8192, 8192),
        ChunkFit::Impossible,
        "equal is not 'fits' — there is no room for the tokenizer to disagree"
    );
}

/// Inside the margin: legal, but the policy will fire sometimes. Warn, do not
/// refuse — the operator may know their content is ASCII-dense.
#[test]
fn target_inside_the_margin_is_tight_not_fatal() {
    // 7500 * 1.15 = 8625 > 8192
    assert_eq!(check_chunk_fit(7500, 8192), ChunkFit::Tight);
}

#[test]
fn target_with_headroom_is_ok() {
    // 6900 * 1.15 = 7935 < 8192
    assert_eq!(check_chunk_fit(6900, 8192), ChunkFit::Ok);
}

/// The check reports; it never rewrites. An operator who sets `num_ctx = 8192`
/// gets 8192 — this is the property the whole design rests on.
#[test]
fn checking_fit_never_alters_the_operators_numbers() {
    let num_ctx = 8192usize;
    let target_chunk_size = 9000usize;
    let verdict = check_chunk_fit(target_chunk_size, num_ctx);

    assert_eq!(verdict, ChunkFit::Impossible);
    assert_eq!(num_ctx, 8192, "num_ctx must be used exactly as written");
    assert_eq!(
        target_chunk_size, 9000,
        "target_chunk_size must not be silently clamped"
    );
}

// ── The BYTE CEILING, not just the target ──────────────────────────────────
//
// `check_chunk_fit` answers the question for `target_chunk_size` only. A target
// can sit comfortably inside the window while the ceiling derived from it does
// not, because the derivation constant is far more generous than a token
// actually costs. Ordinary chunks then embed fine and oversize-split parts —
// cut to exactly that ceiling — cannot.

use vecdb_core::config::{check_ceiling_fit, BYTES_PER_CHUNK_UNIT, MIN_BYTES_PER_TOKEN};

/// A target that fits, whose derived ceiling does not.
#[test]
fn a_fitting_target_can_still_derive_an_impossible_ceiling() {
    let target = 6000usize;
    let num_ctx = 8192usize;

    assert_eq!(
        check_chunk_fit(target, num_ctx),
        ChunkFit::Ok,
        "the target genuinely does fit — this is why the failure was invisible"
    );

    let ceiling = target * BYTES_PER_CHUNK_UNIT;
    assert_eq!(ceiling, 36_000);
    assert_eq!(
        check_ceiling_fit(ceiling, num_ctx),
        ChunkFit::Impossible,
        "36,000 bytes is ~10,300 tokens against an 8,192-token window; the \
         boundary was measured by feeding a model increasing prefixes until it \
         refused."
    );
}

/// The measured boundary recorded on `MIN_BYTES_PER_TOKEN`: at an 8,192-token
/// window, 28,000 bytes embeds and 32,000 does not.
#[test]
fn ceiling_check_matches_the_measured_boundary() {
    assert_eq!(check_ceiling_fit(28_000, 8192), ChunkFit::Ok);
    assert_eq!(check_ceiling_fit(32_000, 8192), ChunkFit::Impossible);
}

/// `BYTES_PER_CHUNK_UNIT` is optimistic relative to what a token actually costs.
/// That gap IS the bug; this pins it so a future edit to either constant has to
/// confront the relationship rather than discover it in production.
#[test]
fn derived_ceiling_is_more_optimistic_than_real_token_cost() {
    assert!(
        (BYTES_PER_CHUNK_UNIT as f64) > MIN_BYTES_PER_TOKEN,
        "if the derivation constant ever drops to the measured floor, the ceiling \
         check becomes redundant — and this test should be revisited, not deleted"
    );
    // A target that fits the window can still derive a ceiling that does not.
    let num_ctx = 8192usize;
    let target = 5000usize;
    assert_eq!(check_chunk_fit(target, num_ctx), ChunkFit::Ok);
    assert_eq!(
        check_ceiling_fit(target * BYTES_PER_CHUNK_UNIT, num_ctx),
        ChunkFit::Impossible
    );
}

/// fastembed reports a sequence limit, so the non-Ollama path can be sized
/// against real capacity. While it reported `None`, ceilings above the model's
/// window went unnoticed — fastembed truncates silently instead of refusing.
#[test]
fn fastembed_models_report_their_sequence_limit() {
    use vecdb_core::embedders::local::fastembed_context_length;

    assert_eq!(
        fastembed_context_length("nomic-embed-text-v1.5"),
        Some(8192)
    );
    assert_eq!(fastembed_context_length("bge-small-en-v1.5"), Some(512));
    assert_eq!(
        fastembed_context_length("all-minilm-l6-v2"),
        Some(256),
        "MiniLM's published sentence-transformers config truncates at 256, \
         below what its BERT backbone could hold"
    );
    assert_eq!(
        fastembed_context_length("something-nobody-has-heard-of"),
        None,
        "an unknown model must report nothing — callers treat None as 'cannot \
         check', never as 'no limit'"
    );

    // A ceiling derived from a large target, against a long-context model.
    let nomic = fastembed_context_length("nomic-embed-text-v1.5").unwrap() as usize;
    assert_eq!(
        check_ceiling_fit(36_000, nomic),
        ChunkFit::Impossible,
        "even an 8,192-token window cannot take a 36,000-byte chunk; fastembed \
         truncates to a prefix rather than erroring, so nothing reports it"
    );
}
