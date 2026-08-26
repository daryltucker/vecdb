"""Where the built binaries actually are.

Every test used to hardcode `./target/debug/<bin>`. That is only correct when
cargo's target directory happens to be `./target`, which it is not here: this
workspace builds ~83 GB with `--all-targets` (ONNX Runtime, CUDA, tree-sitter
grammars, one binary per integration test), so `.cargo/config.toml` redirects
`build.target-dir` onto a larger volume.

The moment it moved, twenty-one test files broke with "Server binary not found"
— pointing at a path no build had written to since the redirect. The failure was
loud, but it was loud about the wrong thing: nothing was wrong with the server.

So the location is resolved rather than assumed. `cargo metadata` is
authoritative — it accounts for `CARGO_TARGET_DIR`, `.cargo/config.toml` at any
level, and the workspace root — and it is the same answer cargo itself uses.
"""

import functools
import json
import os
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


@functools.lru_cache(maxsize=1)
def target_dir() -> Path:
    """Cargo's target directory for this workspace.

    Cached: `cargo metadata` costs ~100 ms and the answer cannot change during a
    run.
    """
    # An explicit env var wins, and asking cargo would return the same thing —
    # but this keeps the common CI override cheap and dependency-free.
    env = os.environ.get("CARGO_TARGET_DIR")
    if env:
        return Path(env)

    try:
        out = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        return Path(json.loads(out.stdout)["target_directory"])
    except (subprocess.CalledProcessError, FileNotFoundError, KeyError, json.JSONDecodeError):
        # Falling back is right — a missing cargo should surface as "binary not
        # built", not as an unrelated crash in path resolution.
        return REPO_ROOT / "target"


def bin_path(name: str, profile: str = "debug") -> str:
    """Absolute path to a built binary.

    Returns a string because every caller passes it straight to `subprocess`.
    """
    return str(target_dir() / profile / name)


def find_bin(name: str) -> str:
    """A built binary, preferring release over debug.

    For tests that only need *a* working binary and would rather use the fast
    one if it happens to exist.
    """
    for profile in ("release", "debug"):
        candidate = target_dir() / profile / name
        if candidate.exists():
            return str(candidate)
    return bin_path(name)
