#!/usr/bin/env python3
"""T0.06 — no test may hardcode cargo's target directory.

`.cargo/config.toml` redirects `build.target-dir` off this volume: the workspace
builds ~83 GB with `--all-targets` (ONNX Runtime, CUDA, tree-sitter grammars, one
binary per integration test), and the root filesystem is a 467 GB NVMe that this
had been quietly filling.

The moment target-dir moved, twenty-six test files broke. Every one of them
failed with a variant of

    Server binary not found at .../target/debug/vecdb-server

which is loud but misleading: nothing was wrong with the server, and the suite
reported a product failure for an environment assumption. Two separate passes
were needed to find them all, because half wrote the literal
`"./target/debug/vecdb"` and half assembled it at runtime with `os.path.join`.

So this is checked rather than remembered. `tests/paths.py` resolves the real
location via `cargo metadata`, which is what cargo itself uses and accounts for
`CARGO_TARGET_DIR` and `.cargo/config.toml` at any level.
"""

import re
import sys
from pathlib import Path

TESTS = Path(__file__).resolve().parent

# Both spellings that were actually found in the tree.
PATTERNS = [
    # "target/debug/vecdb", './target/release/vecq'
    re.compile(r'["\'][^"\']*\btarget/(debug|release)/'),
    # os.path.join(REPO, "target", "debug", ...)  /  ROOT / "target" / "debug"
    re.compile(r'["\']target["\']\s*[,/]\s*["\'](debug|release)["\']'),
]

# This file names the patterns it forbids, and paths.py is the sanctioned resolver.
EXEMPT = {"tier0_target_dir_isolation.py", "paths.py"}


def offending_lines(path: Path):
    out = []
    for n, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
        stripped = line.strip()
        # A comment explaining the rule is not a violation of it.
        if stripped.startswith("#"):
            continue
        if any(p.search(line) for p in PATTERNS):
            out.append((n, stripped))
    return out


def main():
    violations = []
    for path in sorted(list(TESTS.glob("*.py")) + list(TESTS.glob("*.sh"))):
        if path.name in EXEMPT:
            continue
        for n, line in offending_lines(path):
            violations.append(f"  {path.relative_to(TESTS.parent)}:{n}: {line}")

    if violations:
        print(
            "FAIL: tests hardcode cargo's target directory.\n\n"
            + "\n".join(violations)
            + "\n\nBuild output does not live at ./target — .cargo/config.toml\n"
            "redirects it off this volume. A hardcoded path makes the suite report\n"
            "'binary not found' for a healthy build.\n\n"
            "Use tests/paths.py instead:\n"
            "    from paths import bin_path\n"
            "    VECDB = bin_path(\"vecdb\")\n",
            file=sys.stderr,
        )
        sys.exit(1)

    # The resolver has to actually work, or every test that trusts it fails in a
    # way that looks like a missing build.
    sys.path.insert(0, str(TESTS))
    from paths import target_dir

    resolved = target_dir()
    if not resolved.is_absolute():
        print(f"FAIL: paths.target_dir() returned a relative path: {resolved}", file=sys.stderr)
        sys.exit(1)

    print(f"PASS: no hardcoded target paths; resolved target dir = {resolved}")


if __name__ == "__main__":
    main()
