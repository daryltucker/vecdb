//! `respect_gitignore` policy — pinned so it cannot drift again.
//!
//! This has been inverted by an agent and reverted by the operator more than
//! once. The rule, from the head of `CLAUDE.md`:
//!
//!   `.gitignore` is a **build-artifact list, not an indexing policy.**
//!   `.vectorignore` is the knob that governs indexing. `respect_gitignore` is
//!   an escape hatch for people who expect git semantics — never the default,
//!   never inferred.
//!
//! The single permitted exception is the no-`.vectorignore`-anywhere fallback,
//! which must be announced. Both halves are asserted here: a default that
//! silently turns on and a fallback that never fires are different bugs.

use vecdb_core::config::IngestionConfig;
use vecdb_core::ingestion::discovery::resolve_gitignore;
use vecdb_core::ingestion::IngestionOptions;

fn options_at(path: &std::path::Path) -> IngestionOptions {
    IngestionOptions {
        path: path.to_string_lossy().to_string(),
        file_allowlist: None,
        project_root: None,
        collection: "test_gitignore_policy".to_string(),
        vecdbrc_routes: None,
        vecdbrc_root: None,
        target_chunk_size: 512,
        max_chunk_bytes: None,
        on_oversize: Default::default(),
        route_chunking: Default::default(),
        chunk_overlap: 0,
        respect_gitignore: false,
        ignore_vectorignore: false,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
        extensions: None,
        excludes: None,
        dry_run: false,
        metadata: None,
        path_rules: vec![],
        max_concurrent_requests: 1,
        gpu_batch_size: 1,
        quantization: None,
        allow_quantization_delta: false,
    }
}

/// The config default. Not negotiable.
#[test]
fn config_default_is_off() {
    assert!(
        !IngestionConfig::default().respect_gitignore,
        "respect_gitignore must default to FALSE. .gitignore is a build-artifact \
         list, not an indexing policy — .vectorignore governs indexing. If a \
         roadmap, eval, or planning doc says otherwise, the doc is wrong."
    );
    let parsed: IngestionConfig = toml::from_str("target_chunk_size = 512").unwrap();
    assert!(
        !parsed.respect_gitignore,
        "a config omitting the key must not enable it"
    );
}

/// A local `.vectorignore` is an explicit indexing policy: never substitute.
#[test]
fn local_vectorignore_suppresses_the_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".vectorignore"), "*.tmp\n").unwrap();

    let decision = resolve_gitignore(&options_at(dir.path()));
    assert!(
        !decision.respect,
        "with a .vectorignore present, .gitignore must NOT be consulted"
    );
    assert!(!decision.via_fallback);
}

/// The one permitted exception, and it must be flagged as a fallback so the
/// caller can announce it.
#[test]
fn fallback_applies_only_when_no_vectorignore_exists() {
    let dir = tempfile::tempdir().unwrap();
    let decision = resolve_gitignore(&options_at(dir.path()));

    // Guard against a stray ~/.vectorignore on the machine running the tests
    // making this assertion vacuous.
    let home_policy = dirs::home_dir()
        .map(|h| h.join(".vectorignore").exists())
        .unwrap_or(false);

    if home_policy {
        assert!(
            !decision.respect,
            "~/.vectorignore is a policy; do not substitute"
        );
        assert!(!decision.via_fallback);
    } else {
        assert!(
            decision.respect && decision.via_fallback,
            "with no .vectorignore anywhere, .gitignore stands in — and must be \
             reported as a fallback so ingest can say so out loud"
        );
    }
}

/// `--ignore-vectorignore` switches the indexing policy off deliberately.
/// Substituting a different policy is the opposite of what was asked.
#[test]
fn disabling_vectorignore_does_not_summon_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options_at(dir.path());
    opts.ignore_vectorignore = true;

    let decision = resolve_gitignore(&opts);
    assert!(
        !decision.respect,
        "--ignore-vectorignore means no ignore policy, not a different one"
    );
    assert!(!decision.via_fallback);
}

/// Explicitly asking for it still works, and is not mislabelled as a fallback.
#[test]
fn explicit_opt_in_is_honoured_and_not_a_fallback() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".vectorignore"), "*.tmp\n").unwrap();

    let mut opts = options_at(dir.path());
    opts.respect_gitignore = true;

    let decision = resolve_gitignore(&opts);
    assert!(decision.respect, "an explicit request must be honoured");
    assert!(
        !decision.via_fallback,
        "asked-for is not fallback; only the no-policy case is announced"
    );
}
