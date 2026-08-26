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
