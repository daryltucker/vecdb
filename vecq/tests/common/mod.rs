//! Shared by the tests that drive the `vecq` binary.

/// The binary under test: the one cargo just built, unless `VECQ_UNDER_TEST`
/// names another. That override is how a regression test is proved to catch
/// its regression (CLAUDE.md § Evidence: "run the previously-installed binary
/// against the same fixture") — e.g. `VECQ_UNDER_TEST=~/.cargo/bin/vecq cargo
/// test -p vecq --test tier1_refs` must FAIL on a binary without the fix.
pub fn vecq_bin() -> std::path::PathBuf {
    std::env::var_os("VECQ_UNDER_TEST")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_vecq").into())
}
