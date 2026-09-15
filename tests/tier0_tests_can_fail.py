#!/usr/bin/env python3
"""T0.08 — a check that logs a failure must also cause one.

WHAT THIS PROVES
    No test in the manifest reports a problem and then exits 0.

WHY IT EXISTS
    Six gate entries could not fail. Not "did not fail" — *could not*:

      tier1_embedder_config.py   every post-ingest check is a log statement;
                                 `test_local_embedder()` returns True regardless
      tier2_parsers_all.py       the validation block is a literal `pass`
      tier3_audit_cargo.py       collects ANCIENT dependencies, exit commented out
      tier4_agent_workflow.py    idempotency compares counts capped at limit=10,
                                 so the assertion is `10 <= 11` on any real corpus
      tier3_mcp_resources.py     prints on both branches of its last check
      tier4_realistic_ingest.py  prints the files it was supposed to assert on

    Each looked like a test, ran in the gate, and was counted in "58/58".

    CLAUDE.md already says "Ask what would make the test pass without the fix."
    That rule is correct and was violated eight times, which is the argument for
    checking it mechanically rather than writing it down again.

HOW
    Walks the AST of every test in the manifest and flags one shape: an
    `if`/`else` where **both** arms produce output, at least one announces a
    problem (FAIL / FAILURE / ERROR / ❌ / ⚠), and **neither** can make the run
    fail — no `raise`, no `assert`, no `self.fail`, no `sys.exit(nonzero)`, no
    `return`, no `failures.append(...)`, no assignment to a flag like `failed`.

        if len(results) > 0: log(f"✓ Search returned {len(results)}")
        else:                log("⚠ Search returned no results")

    That is the pattern that is always wrong: the test has computed the answer
    and thrown it away.

WHY ONLY THAT SHAPE
    A first cut flagged any single-armed `if` that logged a failure word, and
    reported 28 branches to surface 5 real ones. The noise was all the same
    kind — a display loop printing offending lines just before `return False`,
    or a summary block sitting immediately above `return ok`. Both are correct
    code. Requiring both arms to report, and exempting a block followed by a
    `return`, took it to 4 findings and 0 false positives.

    Messages saying SKIP, Warning: or Note: are exempt — they announce
    something that is deliberately not a failure.

    This does not try to prove a test is GOOD. It proves only that a test which
    has noticed a problem does not then shrug.
"""

import ast
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MANIFEST = REPO / "tests" / "run_all.sh"

# "not found" / "missing" were in a first cut and matched ordinary prose,
# including a success line reading "✓ absent collection reports 'not found'".
ANNOUNCES_FAILURE = re.compile(r"FAIL|FAILURE|ERROR|❌|⚠", re.I)
# A branch may announce something and legitimately carry on: an explicit skip,
# or a warning. Both say so in the message.
NOT_A_FAILURE = re.compile(r"\bSKIP|^\s*(Warning|Note|note):", re.I)
# Names that mean "record this and fail later" — the accumulator pattern
# tier2_pack_granularity.py and tier1_oversize_policy.py use correctly.
ACCUMULATORS = re.compile(r"fail|problem|violation|error|\bok\b", re.I)


def manifest_tests() -> list[Path]:
    """Python tests `run_all.sh` actually runs."""
    text = MANIFEST.read_text()
    names = set(re.findall(r"python3 tests/([A-Za-z0-9_]+\.py)", text))
    return sorted((REPO / "tests" / n) for n in names if (REPO / "tests" / n).exists())


def announces_failure(node: ast.AST) -> str | None:
    """The literal a print/log call in this subtree announces, if any."""
    for n in ast.walk(node):
        if not isinstance(n, ast.Call):
            continue
        fn = n.func
        name = getattr(fn, "id", None) or getattr(fn, "attr", None)
        if name not in ("print", "log", "eprint"):
            continue
        for arg in ast.walk(n):
            if isinstance(arg, ast.Constant) and isinstance(arg.value, str):
                if ANNOUNCES_FAILURE.search(arg.value):
                    return arg.value.strip().splitlines()[0][:70]
    return None


def announces_anything(node: ast.AST) -> bool:
    """Does this subtree print or log at all?"""
    for n in ast.walk(node):
        if isinstance(n, ast.Call):
            name = getattr(n.func, "id", None) or getattr(n.func, "attr", None)
            if name in ("print", "log", "eprint"):
                return True
    return False


def can_fail(node: ast.AST) -> bool:
    """Does this subtree contain a way to make the run fail?"""
    for n in ast.walk(node):
        if isinstance(n, ast.Raise):
            return True
        if isinstance(n, ast.Assert):
            return True
        if isinstance(n, ast.Call):
            fn = n.func
            name = getattr(fn, "attr", None) or getattr(fn, "id", None)
            if name in ("exit", "_exit"):
                # exit(0) is not a failure.
                if n.args and isinstance(n.args[0], ast.Constant):
                    if n.args[0].value in (0, None):
                        continue
                return True
            if name == "fail" or (name or "").startswith("assert"):
                return True
            # failures.append(...) — recorded now, raised at the end.
            if name == "append" and isinstance(fn, ast.Attribute):
                target = getattr(fn.value, "id", "")
                if ACCUMULATORS.search(target):
                    return True
        # Any return hands the decision to the caller — `return None` in
        # tier0_qdrant_isolation, `return 1` in tier0_reset_qdrant, `return
        # False` in a check function. The branch is not shrugging.
        if isinstance(n, ast.Return):
            return True
        # The flag pattern: `failed = True` now, `sys.exit(1 if failed else 0)`
        # at the end. tier2_cli_compliance.py is built this way.
        if isinstance(n, ast.Assign):
            for t in n.targets:
                if isinstance(t, ast.Name) and ACCUMULATORS.search(t.id):
                    return True
    return False


def main() -> int:
    findings = []

    for path in manifest_tests():
        try:
            tree = ast.parse(path.read_text(), filename=str(path))
        except SyntaxError as e:
            findings.append((path.name, e.lineno or 0, f"does not parse: {e.msg}"))
            continue

        # The shape being hunted is an if/else where BOTH arms report and
        # NEITHER fails — "it worked ✓ / it didn't ⚠", then carry on. That is
        # the one pattern that is always wrong.
        #
        # A single-armed `if` that logs is usually a display loop inside a
        # branch that does fail (tier1_vecdbrc_warning prints the offending
        # lines before `return False`), and flagging those buried three real
        # findings in noise.
        for parent in ast.walk(tree):
            body = getattr(parent, "body", None)
            if not isinstance(body, list):
                continue
            for i, node in enumerate(body):
                if not isinstance(node, ast.If) or not node.orelse:
                    continue
                arms = [node.body, node.orelse]
                shouts = [
                    next(
                        (s for s in (announces_failure(st) for st in arm) if s),
                        None,
                    )
                    for arm in arms
                ]
                # At least one arm must announce a problem, and both must
                # produce output — otherwise this is an ordinary branch.
                bad = next(
                    (s for s in shouts if s and not NOT_A_FAILURE.search(s)), None
                )
                if not bad or not all(
                    any(announces_anything(st) for st in arm) for arm in arms
                ):
                    continue
                if any(can_fail(st) for arm in arms for st in arm):
                    continue
                # A reporting block immediately before the decision is fine:
                #   if ok: log("PASSED") else: log("FAILED")
                #   return ok
                nxt = body[i + 1] if i + 1 < len(body) else None
                if isinstance(nxt, ast.Return) or (
                    nxt is not None and can_fail(nxt)
                ):
                    continue
                findings.append((path.name, node.lineno, f"both arms log, neither fails: {bad}"))

    if findings:
        print(
            "FAIL: branches that announce a problem and then continue:\n",
            file=sys.stderr,
        )
        for name, line, msg in sorted(findings):
            print(f"  {name}:{line}: {msg}", file=sys.stderr)
        print(
            f"\n{len(findings)} branch(es). A test that reports a failure must cause one —\n"
            f"otherwise it is counted as a pass and the defect ships.",
            file=sys.stderr,
        )
        return 1

    print(f"PASS: every failure a manifest test announces also fails the run.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
