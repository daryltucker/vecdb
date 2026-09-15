#!/usr/bin/env python3
"""T2.10 — every repo path a shipped file names must exist in a clone.

WHAT THIS PROVES
    Paths referenced from tracked files resolve to tracked files. Two failure
    classes, reported separately because they need different fixes:

      PRIVATE  the target exists on this disk but git ignores it, so a clone
               gets a dead link and the reference advertises the private tree
      MISSING  nothing is there at all, on any machine

WHY IT EXISTS
    `.gitignore` starts with `/*` and line 10 is `**/docs/**/`, un-ignoring only
    `docs/vecq` and `docs/specs`. So `docs/planning/`, `docs/internal/`,
    `docs/inquiries/`, `docs/training/` are deliberately present in a working
    tree and absent from every clone. Thirteen tracked files linked into them,
    `README.md` among them — sending users to a file their clone does not have
    for the answer to a build problem.

    The MISSING class is worse. `install.sh` runs `scripts/prune_target.sh`
    under `set -e`; there is no `scripts/` directory and `.gitignore`'s
    allowlist would never permit one, so the script aborts on its first real
    statement — and `README.md` makes it the first Quick Start command. Three
    other files reference `scripts/` too.

    Both classes are invisible to a human reader on the machine where the files
    happen to exist. That is the whole point: this check asks git, not the
    filesystem.

SCOPE
    Deliberately narrow, because a check that cries wolf gets switched off.
    Only two shapes are examined:

      1. markdown links with a relative target — an unambiguous claim that a
         file is there
      2. inline path-like tokens whose first segment is a real top-level
         directory of this repo

    An example path in a code fence (`src/main.rs`, `/path/to/thing`) matches
    neither and is left alone.
"""

import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def git(*args) -> list[str]:
    out = subprocess.run(
        ["git", *args], cwd=REPO, capture_output=True, text=True, check=True
    )
    return [l for l in out.stdout.splitlines() if l]


TRACKED = set(git("ls-files"))
# Every directory that holds at least one tracked file, so a reference to a
# directory (`docs/specs/`) can be resolved as well as one to a file.
TRACKED_DIRS = {str(Path(p).parent) for p in TRACKED} | {"."}
for d in list(TRACKED_DIRS):
    p = Path(d)
    while str(p) not in (".", "/"):
        TRACKED_DIRS.add(str(p))
        p = p.parent
TOP_LEVEL = {p.split("/")[0] for p in TRACKED if "/" in p}

# Text files only. A path inside a JSON fixture is data, not a reference.
CHECKABLE = tuple(".md .rs .py .sh .toml Makefile Dockerfile".split())

# A path that is DATA rather than a reference — a sample fed to the detector,
# a placeholder in an error message — opts out explicitly. Exceptions have to be
# written down and are greppable, same as the privacy scanner's `privacy-ok:`.
PATH_OK = re.compile(r"(?://|#)\s*path-ok:")


MD_LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
# A path-ish token: at least one slash, a file extension or a trailing slash.
INLINE = re.compile(r"(?<![\w/.-])((?:\.\./|\./)?[\w.-]+(?:/[\w.-]+)+/?)")

# Not repo paths.
def is_external(ref: str) -> bool:
    return (
        "://" in ref
        or ref.startswith(("/", "~", "#", "$", "mailto:"))
        or any(c in ref for c in "<>{}*$|")
        # Documentation placeholders.
        or "path/to" in ref
        or "yourusername" in ref
        or ref.startswith(("http", "www."))
    )


# Trees a test builds at runtime, and fixtures `tests/fixtures/init.sh`
# fetches. Referencing these is correct; they are absent from a clone on
# purpose and appear when the suite runs.
RUNTIME_CREATED = (
    "tests/run/",
    "tests/fixtures/external/",
    "tests/fixtures/git_test_repo",
    "tests/fixtures/inc_test_repo",
    "tests/tier1_parsers_test_env",
)


def resolve(ref: str, from_file: str) -> str | None:
    """Repo-relative form of `ref` as written in `from_file`, or None.

    Markdown links resolve against the containing directory — that is what a
    link means. A first cut treated a bare `CONFIG.md` in `docs/CLI.md` as
    repo-root-relative and reported 60 sibling links as missing, which is the
    kind of noise that gets a check deleted.
    """
    # Sentence punctuation swept up by the inline pattern: a trailing full stop
    ref = ref.split("#", 1)[0].split("?", 1)[0].strip().rstrip(".,:;)").rstrip("/")
    if not ref or is_external(ref):
        return None

    base = Path(from_file).parent

    # Two readings, and either resolving is enough.
    #
    # In markdown a link is relative to its file. In source, an `include_str!`
    # argument is relative too, while a doc comment naming a path from the top
    # of the tree means the repo root. Both spellings are legitimate, and a
    # checker that insists on one reports the other as broken.
    candidates = []
    if ref.startswith(("./", "../")):
        candidates.append(REPO / base / ref)
    else:
        candidates.append(REPO / base / ref)
        candidates.append(REPO / ref)

    resolved = []
    for c in candidates:
        try:
            resolved.append(os.path.relpath(c.resolve(), REPO))
        except (OSError, ValueError):
            continue
    if not resolved:
        return None
    # A reading that resolves wins. Failing that, prefer the one with the more
    # actionable verdict: an ignored-but-present path named from a .rs doc
    # comment is a root-relative reference into the private tree (PRIVATE), and
    # reporting it under its relative reading as MISSING would send someone
    # looking for a file that was never meant to be there.
    for r in resolved:
        if classify(r) is None:
            return r
    for r in resolved:
        if classify(r) == "PRIVATE":
            return r
    return resolved[0]


def classify(rel: str) -> str | None:
    """None if fine, else 'PRIVATE' or 'MISSING'."""
    if rel in TRACKED or rel in TRACKED_DIRS:
        return None
    if rel.startswith("..") or rel.startswith("/"):  # escaped the repo entirely
        return None
    if rel.startswith(RUNTIME_CREATED):
        return None
    # No extension and not a tracked directory: a Cargo feature spec
    # (`vecdb-core/cuda`), a URL fragment, or prose. Not a path claim.
    if "." not in Path(rel).name:
        return None
    on_disk = (REPO / rel).exists()
    return "PRIVATE" if on_disk else "MISSING"


def references(path: str, text: str):
    """(reference, line_number) pairs worth resolving.

    Fenced code blocks are skipped. They hold sample output and shell
    transcripts — `# Output: "Build Instructions -> docs/BUILDING.md"` is a
    demonstration of vecq's link extractor, not a link.
    """
    fenced = False
    for n, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            continue
        if fenced or PATH_OK.search(line):
            continue
        for m in MD_LINK.finditer(line):
            yield m.group(1), n
        for m in INLINE.finditer(line):
            ref = m.group(1)
            # Only claims about THIS repo's layout.
            if ref.split("/")[0] in TOP_LEVEL:
                yield ref, n


def main() -> int:
    private, missing = [], []

    for f in sorted(TRACKED):
        # Fixture bodies are DATA. A path inside them is sample content.
        if "/fixtures/" in f or f.startswith("tests/fixtures/"):
            continue
        if not f.endswith(CHECKABLE) and Path(f).name not in ("Makefile", "Dockerfile"):
            continue
        try:
            text = (REPO / f).read_text(errors="replace")
        except OSError:
            continue
        seen = set()
        for ref, line in references(f, text):
            rel = resolve(ref, f)
            if rel is None or (f, rel) in seen:
                continue
            seen.add((f, rel))
            verdict = classify(rel)
            if verdict == "PRIVATE":
                private.append(f"{f}:{line}: {ref}  -> {rel}")
            elif verdict == "MISSING":
                missing.append(f"{f}:{line}: {ref}  -> {rel}")

    if private:
        print("FAIL: tracked files reference paths git does not ship:", file=sys.stderr)
        print(
            "      (present on THIS disk, absent from every clone — "
            "check `git check-ignore -v <path>`)\n",
            file=sys.stderr,
        )
        for v in private:
            print(f"  PRIVATE  {v}", file=sys.stderr)
        print(file=sys.stderr)

    if missing:
        print("FAIL: tracked files reference paths that do not exist:", file=sys.stderr)
        for v in missing:
            print(f"  MISSING  {v}", file=sys.stderr)
        print(file=sys.stderr)

    if private or missing:
        print(
            f"{len(private)} private, {len(missing)} missing. A shipped file may only\n"
            f"name a path a clone actually has.",
            file=sys.stderr,
        )
        return 1

    print(f"PASS: every repo path named by {len(TRACKED)} tracked files resolves.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
