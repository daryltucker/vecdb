#!/usr/bin/env python3
"""Every tier must test the binary this tree just built — and keep testing it.

WHAT THIS PROVES
    The binaries the suite shells out to report the same git revision as the
    working tree, and every test resolves to the same one.

WHY IT EXISTS
    A green run only means something if you know what it ran. The suite had
    four different answers to "the binary" at once:

      * 28 tests via `paths.bin_path()`      -> target/debug/vecdb
      * T2.7c, T2.8 via `paths.find_bin()`   -> target/release/vecdb, which the
                                                gate never builds
      * T2.7b via a bare "vecdb" on PATH     -> ~/.cargo/bin/vecdb, installed
                                                whenever someone last ran
                                                `make install`
      * GPU assertions                       -> cuda-dynamic or default,
                                                depending on position

    Measured on one working tree: debug 21:10, release 20:20, installed 17:37.
    Three different builds, one "PASS".

    The feature-level half is just as quiet. `cargo test -p vecdb-cli` and
    `cargo run --bin vecdb` both REBUILD `vecdb` with default features at the
    same path T1.1 wrote the cuda-dynamic build to. That happens at five points
    in the manifest, and `run_all.sh` re-asserts the supported build after
    exactly one of them — so the tiers named "Reality" and "Agent Reality" ran
    CPU-only while claiming to cover the operator's GPU route.

    Revision is the right thing to compare because `vecdb-common/build.rs`
    already stamps it, `--version` already prints it, and it is the one value
    that cannot be satisfied by an accidentally-correct binary.

USAGE
    python3 tests/tier0_binary_provenance.py [label]

    Called once after the build, and again at each tier boundary. The label is
    only for the message, so a failure says WHERE the binary drifted.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import bin_path, expected_revision, find_bin, revision_of, target_dir

BINARIES = ("vecdb", "vecdb-server", "vecq")


def log(msg, status="INFO"):
    prefix = {"PASS": "[PASS]", "FAIL": "[FAIL]", "WARN": "[WARN]"}.get(status, "[INFO]")
    print(f"{prefix} {msg}", file=sys.stderr)


def main() -> int:
    label = sys.argv[1] if len(sys.argv) > 1 else "build"

    want = expected_revision()
    if want is None:
        # No git checkout: a source tarball stamps "unknown" and there is
        # nothing to compare against. Not a failure — see version.rs's
        # `a_revision_is_either_a_hash_or_honestly_unknown`.
        log("not a git checkout — revision cannot be compared, skipping.", "WARN")
        return 0

    failures = []

    # 1. One resolver, one answer. `find_bin` preferring release is how two
    #    gate entries came to test a binary the gate never built.
    for name in BINARIES:
        if find_bin(name) != bin_path(name):
            failures.append(
                f"{name}: find_bin() and bin_path() disagree "
                f"({find_bin(name)} vs {bin_path(name)}). The suite must have "
                f"exactly one idea of which binary is under test."
            )

    # 2. A stale release build must not sit where a test could reach it.
    #    Reported rather than failed: it is not wrong to have one, only wrong
    #    to silently prefer it.
    for name in BINARIES:
        rel = target_dir() / "release" / name
        if rel.exists():
            rel_rev = revision_of(str(rel))
            if rel_rev != want:
                log(
                    f"{name}: a release build exists at {rel} reporting "
                    f"{rel_rev or 'no revision'} (tree is {want}). Nothing uses it; "
                    f"`cargo clean --release -p <crate>` if that is surprising.",
                    "WARN",
                )

    # 3. The binaries under test report this tree.
    for name in BINARIES:
        path = bin_path(name)
        if not os.path.exists(path):
            failures.append(f"{name}: not built at {path}")
            continue
        got = revision_of(path)
        if got is None:
            failures.append(
                f"{name}: {path} did not report a revision. `--version` must "
                f"print `(git:<hash>)`; vecdb-common/build.rs stamps it."
            )
        elif got != want:
            failures.append(
                f"{name}: built binary reports git:{got}, working tree is git:{want}.\n"
                f"         Something rebuilt or replaced {path} mid-run.\n"
                f"         `cargo test -p vecdb-cli` and `cargo run --bin vecdb` both do this."
            )

    if failures:
        log(f"binary provenance FAILED at '{label}':", "FAIL")
        for f in failures:
            log(f"  {f}", "FAIL")
        log("A suite that cannot say which binary it ran cannot report a result.", "FAIL")
        return 1

    log(f"[{label}] {', '.join(BINARIES)} all report git:{want}", "PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
