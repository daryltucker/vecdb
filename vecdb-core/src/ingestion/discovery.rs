use crate::ingestion::IngestionOptions;
use ignore::WalkBuilder;
use std::path::Path;

/// Whether `.gitignore` will be consulted for this walk, and why.
pub struct GitignoreDecision {
    pub respect: bool,
    /// True only when `.gitignore` is being used as a stand-in because no
    /// `.vectorignore` exists. Callers must say so in their output — this is the
    /// one case where the walk honours a file the operator did not point at.
    pub via_fallback: bool,
}

/// Resolve whether `.gitignore` applies.
///
/// **`respect_gitignore` is never the default and is never inferred.**
/// `.gitignore` is a build-artifact list, not an indexing policy; the two
/// disagree constantly, and honouring it silently drops content that should be
/// indexed. `.vectorignore` is the knob that governs indexing. Do not "fix" this
/// — see the standing rule at the head of `CLAUDE.md`.
///
/// The single permitted exception: if there is **no `.vectorignore` anywhere**
/// — neither at the ingest root nor `~/.vectorignore` — then the operator has
/// expressed no indexing policy at all, and walking straight into `target/` or
/// `node_modules/` serves nobody. In that case, and only that case, `.gitignore`
/// stands in. It is announced, never silent.
pub fn resolve_gitignore(options: &IngestionOptions) -> GitignoreDecision {
    // Explicitly asked for: honour it, and it is not a fallback.
    if options.respect_gitignore {
        return GitignoreDecision {
            respect: true,
            via_fallback: false,
        };
    }

    // `--ignore-vectorignore` means the operator deliberately switched the
    // indexing policy off. Substituting a different one is the opposite of what
    // they asked for.
    if options.ignore_vectorignore {
        return GitignoreDecision {
            respect: false,
            via_fallback: false,
        };
    }

    // The ingest root, or its parent when the target is a single file.
    let root = Path::new(&options.path);
    let local = if root.is_file() {
        root.parent().unwrap_or(root).join(".vectorignore")
    } else {
        root.join(".vectorignore")
    };

    let has_policy = local.exists()
        || dirs::home_dir()
            .map(|h| h.join(".vectorignore").exists())
            .unwrap_or(false);

    GitignoreDecision {
        respect: !has_policy,
        via_fallback: !has_policy,
    }
}

pub fn build_walker(options: &IngestionOptions) -> WalkBuilder {
    let respect_gitignore = resolve_gitignore(options).respect;

    let mut builder = WalkBuilder::new(&options.path);
    builder
        .git_ignore(respect_gitignore)
        .ignore(respect_gitignore)
        .hidden(false);

    if !options.ignore_vectorignore {
        builder.add_custom_ignore_filename(".vectorignore");
    }

    // Always exclude configuration/ignore files — they are not ingestable content
    //
    // `.git` is excluded for a different reason than `.gitignore` is consulted:
    // this is not an ignore policy, it is the repository's internal database.
    // Reflogs, refs, hook samples and `COMMIT_EDITMSG` are storage, not source,
    // and they are worse than merely useless in an index — commit-message drafts
    // paraphrase the code, so they rank *above* the file they describe. Observed
    // directly: a search for "Version 2" returned `.git/COMMIT_EDITMSG` at 0.798
    // ahead of the `main.rs` that actually contained it at 0.736.
    //
    // `.hidden(false)` above is deliberate — dotfiles are often real content —
    // so this exclusion has to be explicit.
    builder.filter_entry(move |entry| {
        let name = entry.file_name();
        name != ".vecdbrc" && name != ".vectorignore" && name != ".gitignore" && name != ".git"
    });

    builder
}

pub fn count_files(builder: &WalkBuilder) -> u64 {
    let count_walker = builder.build();
    count_walker
        .filter_map(|r| r.ok())
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| !e.path().components().any(|c| c.as_os_str() == ".vecdb"))
        .count() as u64
}
