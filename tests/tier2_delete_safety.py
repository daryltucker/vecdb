#!/usr/bin/env python3
"""T2.6f — `vecdb delete` must never act on, or lie about, an endpoint it did
not actually reach.

Two defects, both found 2026-257, both about the same thing: the endpoint the
command REPORTS on must be the endpoint it USED.

WHAT THIS PROVES
    1. The `--all` locality guard is evaluated against the endpoint the
       deletion will actually use — not a different one resolved earlier from
       different flags.
    2. An unreachable store is reported as unreachable, never as "collection
       not found".

WHY IT EXISTS (B2, audit 2026-257)
    The endpoint was resolved twice. `cli.rs` resolved from the GLOBAL
    `--profile` and checked the guard against that. `commands/delete.rs`
    re-resolved from the SUBCOMMAND-LOCAL `-P` and applied `--url` on top, then
    deleted against that second answer.

    The exploitable divergence was `--url`:

        vecdb delete --all --url http://<remote>:PORT

    `--url` is applied inside delete.rs, AFTER cli.rs has already run the guard,
    so the guard never saw it at all. It read the default profile's localhost,
    passed, and the command then went on to list and delete every collection on
    the remote host. No trickery — documented flags behaving as documented.

    CORRECTION TO THE AUDIT. The audit also named `delete --all -P <remote>` as
    a bypass. Measured against the pre-fix binary, it is NOT: the root
    `--profile` is declared `global = true` (cli.rs), so `-P` populates BOTH
    `cli.profile` and delete's own `args.profile`. They cannot be given
    different values, the guard saw the remote, and it refused. That route is
    asserted below anyway — it was correct by accident, and the fix must not
    quietly lose it.

    Fixed by resolving once, in delete.rs, with the guard immediately after
    `--url` — so there is no second answer for a guard to disagree with.

WHY PART 2 EXISTS
    `backends/qdrant.rs::collection_exists` was `collection_info(name)` with
    `Err(_) => Ok(false)`. Unreachable host, refused connection, TLS failure,
    bad API key and genuine absence all returned `false`. Measured against
    192.0.2.1 before the fix:

        $ vecdb delete some_collection -P <remote profile> --force
        Deleting 'some_collection' at http://192.0.2.1:19333... not found
          — nothing deleted
        $ echo $?
        0

    Exit 0, and the operator is told the data is already gone by a command that
    never contacted the store holding it. delete.rs had a correct
    "could not reach store" branch the whole time; the swallowed error made it
    unreachable code.

HOW TO PROVE THIS TEST CATCHES THE REGRESSIONS
    Part 1: restore the two-resolution shape (guard in `cli.rs`,
    `config.resolve` in `delete.rs`). The `--url` case stops bailing and
    proceeds toward the network instead.
    Part 2: restore `Err(_) => Ok(false)`. The unreachable case reports
    "not found" and exits 0.
    Both were confirmed to fail against the pre-fix binary before this file was
    registered.

WHY IT NEEDS NO QDRANT
    The guard bails before `Core::new`, so nothing connects. The fixture's
    `remote_guard` profile points at 192.0.2.1 (TEST-NET-1, RFC 5737), which is
    guaranteed not to route — a test asserting "does not connect" must be
    unable to connect.
"""

import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import require_bin

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "tests", "fixtures", "config.toml")

# TEST-NET-1, on a port that is deliberately NOT 6333/6334 — T0.0 rejects those
# anywhere in the fixture, by port alone. Must match [profiles.remote_guard].
REMOTE_URL = "http://192.0.2.1:19333"

# The guard's message. Asserted on so that a *different* failure — a missing
# binary, a config error, a panic — cannot be mistaken for the guard working.
GUARD_TEXT = "restricted to local backends"


def run(args, timeout=60):
    """Run vecdb with the shared fixture config. Never the operator's."""
    env = dict(os.environ)
    env["VECDB_CONFIG"] = FIXTURE
    return subprocess.run(
        [require_bin("vecdb"), *args],
        capture_output=True,
        text=True,
        timeout=timeout,
        env=env,
        cwd=REPO,
    )


def expect_refused(label, args):
    r = run(args)
    out = r.stdout + r.stderr
    if r.returncode == 0:
        raise SystemExit(
            f"FAIL [{label}]: `delete --all` exited 0 against {REMOTE_URL}.\n"
            f"  argv: {' '.join(args)}\n"
            f"  The guard did not stop a remote bulk deletion.\n"
            f"--- output ---\n{out}"
        )
    if GUARD_TEXT not in out:
        raise SystemExit(
            f"FAIL [{label}]: exited {r.returncode}, but not via the locality "
            f"guard — {GUARD_TEXT!r} absent.\n"
            f"  argv: {' '.join(args)}\n"
            f"  A non-zero exit for some other reason is not this guard "
            f"working.\n--- output ---\n{out}"
        )
    if REMOTE_URL not in out:
        raise SystemExit(
            f"FAIL [{label}]: guard fired but did not name the endpoint it "
            f"refused. An operator cannot diagnose a refusal that does not say "
            f"what it refused.\n--- output ---\n{out}"
        )
    print(f"  ✅ {label}: refused, naming {REMOTE_URL}")


def expect_allowed_to_reach_local(label, args):
    """A LOCAL `--all` must NOT be stopped by the guard.

    A guard that refuses everything is not a guard, it is a broken command, and
    it would make the two assertions above pass for the wrong reason. This runs
    without --force, so the interactive token prompt aborts it on a closed
    stdin — the deletion never happens, but the guard is proven not to have
    fired.
    """
    r = run(args)
    out = r.stdout + r.stderr
    if GUARD_TEXT in out:
        raise SystemExit(
            f"FAIL [{label}]: the locality guard refused a LOCAL endpoint.\n"
            f"  argv: {' '.join(args)}\n--- output ---\n{out}"
        )
    print(f"  ✅ {label}: guard correctly stood aside")


def main():
    print("=== T2.6f: delete --all locality guard ===")

    # Route 1: the subcommand-local -P. Correct even before the fix (the root
    # --profile is `global = true`, so -P sets both), asserted so the
    # single-resolution rewrite cannot lose behaviour it already had.
    expect_refused("subcommand -P <remote>", ["delete", "--all", "-P", "remote_guard"])
    expect_refused(
        "subcommand --profile <remote>", ["delete", "--all", "--profile", "remote_guard"]
    )

    # Route 2: --url. THE REAL BYPASS — applied inside delete.rs, after cli.rs
    # had already run the guard, so the guard never saw it. Confirmed failing
    # against the pre-fix binary: it did not bail, it tried to connect.
    expect_refused("--url <remote>", ["delete", "--all", "--url", REMOTE_URL])

    # Route 3: the global --profile, which delete used to ignore entirely.
    # Before the fix this was the ONE route the guard did inspect; it is
    # asserted so the fix cannot regress it while fixing the others.
    expect_refused(
        "global --profile <remote>",
        ["--profile", "remote_guard", "delete", "--all"],
    )

    # Route 4: both set. The subcommand's -P wins, so a remote -P must still be
    # refused even when the global profile is local — this is the exact shape of
    # the original bypass.
    expect_refused(
        "global local + subcommand -P remote",
        ["--profile", "tier1_basic", "delete", "--all", "-P", "remote_guard"],
    )

    # The control.
    expect_allowed_to_reach_local(
        "local profile", ["--profile", "tier1_basic", "delete", "--all"]
    )

    # ── Part 2: an unreachable store is never reported as "not found" ──
    #
    # A single named collection, so the `--all` guard is not involved at all:
    # this is the existence check, on its own. `--force` skips the token prompt
    # so the command runs to the point where it decides what to report.
    print("\n--- unreachable store must not report 'not found' ---")
    r = run(["delete", "test_probe_absent", "-P", "remote_guard", "--force"], timeout=180)
    out = r.stdout + r.stderr

    if "not found" in out:
        raise SystemExit(
            "FAIL: an UNREACHABLE store was reported as 'not found — nothing "
            "deleted'.\n"
            "  The operator is being told their data is already gone by a "
            "command that never reached the store.\n"
            f"  exit={r.returncode}\n--- output ---\n{out}"
        )
    if r.returncode == 0:
        raise SystemExit(
            f"FAIL: exit 0 against an unreachable store ({REMOTE_URL}). "
            "A delete that could not ask must not succeed.\n"
            f"--- output ---\n{out}"
        )
    if "could not reach store" not in out:
        raise SystemExit(
            "FAIL: non-zero exit, but the failure does not say the store was "
            "unreachable. Diagnosing this requires naming the real cause.\n"
            f"  exit={r.returncode}\n--- output ---\n{out}"
        )
    print(f"  ✅ unreachable store: reported as unreachable, exit {r.returncode}")

    print("\nPASS: remote bulk deletion refused by every route; "
          "unreachable never reported as absent")


if __name__ == "__main__":
    main()
