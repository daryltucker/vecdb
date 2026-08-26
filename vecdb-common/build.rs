//! Stamp the git revision into the binary at compile time.
//!
//! The three call sites this replaces each ran `git rev-parse` at *runtime*,
//! with `.current_dir()` hardcoded to the author's checkout. That fails three
//! ways at once: it reports "unknown" on any other machine, it leaks a home
//! path into a shipped binary, and where the path does exist it reports
//! whatever is checked out *now* rather than what the binary was built from —
//! so an installed binary silently tracks a moving target.
//!
//! Build time is the only moment the answer is knowable and fixed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // An explicit override always wins: distro packagers, reproducible builds,
    // and vendored source drops can stamp a known value with no git checkout.
    println!("cargo:rerun-if-env-changed=VECDB_GIT_HASH");
    if let Ok(explicit) = std::env::var("VECDB_GIT_HASH") {
        if !explicit.trim().is_empty() {
            println!("cargo:rustc-env=VECDB_GIT_HASH={}", explicit.trim());
            println!("cargo:rustc-env=VECDB_GIT_DIRTY=0");
            return;
        }
    }

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));

    if let Some(git_dir) = find_git_dir(&manifest_dir) {
        emit_rerun_directives(&git_dir);
    }

    // A source tarball or a vendored crate has no .git. That is not an error;
    // it just means the revision is not knowable, and "unknown" is honest.
    let hash = git(&manifest_dir, &["rev-parse", "--short", "HEAD"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    // Whether the tree had uncommitted changes when this was built. A bare
    // hash on a dirty tree names a commit whose contents are not what is
    // running, which is how a "fixed in abc123" report survives a rebuild.
    // Untracked files are excluded deliberately: a scratch file does not change
    // what the binary does, and counting it would mark nearly every dev build.
    let dirty = git(
        &manifest_dir,
        &["status", "--porcelain", "--untracked-files=no"],
    )
    .map(|s| !s.is_empty())
    .unwrap_or(false);

    println!("cargo:rustc-env=VECDB_GIT_HASH={hash}");
    println!(
        "cargo:rustc-env=VECDB_GIT_DIRTY={}",
        if dirty { "1" } else { "0" }
    );
}

/// Run a git command in `dir`, returning trimmed stdout on success.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_string())
}

/// Walk up from `start` looking for a `.git` entry.
///
/// Handles both a normal `.git/` directory and the `gitdir: <path>` pointer
/// file git writes for linked worktrees and submodules. The previous version
/// hardcoded `../.git/HEAD`, which resolves to nothing inside a worktree — so
/// the rebuild triggers silently stopped firing exactly where an agent is most
/// likely to be working.
fn find_git_dir(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let candidate = ancestor.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.is_file() {
            let contents = std::fs::read_to_string(&candidate).ok()?;
            let target = contents.strip_prefix("gitdir:")?.trim();
            let resolved = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                ancestor.join(target)
            };
            return resolved.exists().then_some(resolved);
        }
    }
    None
}

/// Emit `cargo:rerun-if-changed` for the files that determine HEAD's hash.
///
/// LIMITATION (deliberate, documented):
/// Cargo's rerun-if-changed is file-mtime based, so this is best-effort:
///
/// - `.git/HEAD` — changes on branch switch and detached-HEAD moves.
/// - the loose ref HEAD points at — e.g. `.git/refs/heads/main`; changes on
///   commit. **This is the one that matters**, and watching `.git/index`
///   instead only worked by accident, because committing happens to rewrite
///   the index too.
/// - `.git/packed-refs` — after `git gc` the loose ref file is deleted and the
///   tip lives here.
///
/// In a freshly-packed repository the recorded hash can still go stale until
/// something else rebuilds this crate. The alternative — an unconditional
/// rerun every invocation — costs far more than an occasionally stale hash in
/// a version banner. For a guaranteed-exact value pass `VECDB_GIT_HASH`
/// explicitly, or `cargo clean -p vecdb-common` first.
fn emit_rerun_directives(git_dir: &Path) {
    let head = git_dir.join("HEAD");
    if !head.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", head.display());

    let packed = git_dir.join("packed-refs");
    if packed.exists() {
        println!("cargo:rerun-if-changed={}", packed.display());
    }

    // If HEAD is symbolic ("ref: refs/heads/main"), also watch the loose ref.
    // A detached HEAD writes the raw SHA into HEAD itself, already covered.
    let Ok(contents) = std::fs::read_to_string(&head) else {
        return;
    };
    if let Some(ref_path) = contents.strip_prefix("ref:") {
        let loose = git_dir.join(ref_path.trim());
        if loose.exists() {
            println!("cargo:rerun-if-changed={}", loose.display());
        }
    }
}
