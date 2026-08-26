//! Build-time version identity, shared by every binary in the workspace.
//!
//! Stamped by `build.rs` at compile time rather than read from `git` at
//! runtime. See that file for why the runtime form was wrong.

/// Short git revision this binary was built from, or `"unknown"` when built
/// outside a git checkout (a source tarball, a vendored crate, a Docker build
/// that did not copy `.git`).
pub const GIT_HASH: &str = match option_env!("VECDB_GIT_HASH") {
    Some(h) => h,
    // Defensive: build.rs always emits this, but `option_env!` keeps the crate
    // compilable if the build script is ever skipped — `env!` would refuse to
    // build at all, turning a missing stamp into a broken package.
    None => "unknown",
};

/// Whether the working tree had uncommitted tracked changes at build time.
///
/// Untracked files are excluded deliberately: a stray scratch file does not
/// change what the binary does, and treating it as dirty would mark almost
/// every development build.
pub const GIT_DIRTY: bool = matches!(
    match option_env!("VECDB_GIT_DIRTY") {
        Some(d) => d,
        None => "0",
    }
    .as_bytes(),
    b"1"
);

/// `"6d62c2f"`, or `"6d62c2f-dirty"` when built from a modified tree.
///
/// The suffix matters when someone reports a bug against a hash: without it, a
/// build containing uncommitted work is indistinguishable from the commit it
/// claims to be.
pub fn revision() -> String {
    if GIT_DIRTY {
        format!("{GIT_HASH}-dirty")
    } else {
        GIT_HASH.to_string()
    }
}

/// One line: `vecdb v1.0.4 (git:6d62c2f)`.
pub fn short_version(name: &str, pkg_version: &str) -> String {
    format!("{name} v{pkg_version} (git:{})", revision())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point is that this is baked in, not discovered at runtime.
    /// An empty value would render as `(git:)` and read like a bug in the
    /// caller rather than a missing stamp.
    #[test]
    fn hash_is_always_populated() {
        assert!(!GIT_HASH.is_empty());
        assert!(!revision().is_empty());
    }

    /// A revision is either a real hex hash or the honest literal "unknown".
    ///
    /// This deliberately does NOT assert `!= "unknown"`. That assertion held in
    /// a git checkout and failed by design in a source tarball — which is
    /// precisely the artifact crates.io ships, so it would have broken
    /// `cargo test` for everyone who installed from the registry while passing
    /// for everyone who could have noticed.
    #[test]
    fn a_revision_is_either_a_hash_or_honestly_unknown() {
        assert!(
            GIT_HASH == "unknown" || GIT_HASH.chars().all(|c| c.is_ascii_hexdigit()),
            "not a hex revision and not the documented fallback: {GIT_HASH}"
        );
    }

    /// `git rev-parse` emits a trailing newline; build.rs trims it. An
    /// untrimmed value renders as a broken banner rather than an error.
    #[test]
    fn the_hash_carries_no_whitespace() {
        assert_eq!(GIT_HASH, GIT_HASH.trim());
        assert!(!GIT_HASH.contains(char::is_whitespace), "{GIT_HASH:?}");
    }

    #[test]
    fn short_version_names_both_the_release_and_the_commit() {
        let s = short_version("vecdb", "1.0.4");
        assert!(s.starts_with("vecdb v1.0.4 (git:"), "{s}");
        assert!(s.ends_with(')'), "{s}");
    }
}
