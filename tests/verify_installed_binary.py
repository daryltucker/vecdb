#!/usr/bin/env python3
"""P.3 — the INSTALLED binary is the one this tree builds.

WHAT THIS PROVES
    `~/.cargo/bin/vecdb` and `~/.cargo/bin/vecdb-server` report the same git
    revision as the working tree, and the server still answers MCP.

WHY IT CHANGED
    This asserted one thing: that `vecdb://manual` appears in `resources/list`.
    Then it printed "vecdb://manual found! Binary is updated. ✅".

    A resource existing says nothing about which build is installed. Measured on
    one tree: the suite built `target/debug/vecdb` at 21:10 and P.3 reported the
    installed binary "updated" while `~/.cargo/bin/vecdb` was from 17:37, four
    hours and one commit behind — and that is the binary the operator's nightly
    cron actually executes. Every fix the suite had just verified was absent
    from the thing in production use.

    Revision is the right comparison: vecdb-common/build.rs stamps it, and
    nothing else distinguishes "installed after the fix" from "installed
    before".

WHEN THIS FAILS
    It means you have not installed what you just tested. That is a real
    condition, not a nuisance — run `make install` (or `make install-cuda-dynamic`
    on a BYO-ONNX-Runtime machine) and re-run.
"""

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import expected_revision, revision_of
from lib_stdio import drain_stderr

INSTALL_ROOT = os.path.expanduser("~/.cargo/bin")
REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "tests", "fixtures", "config.toml")


class InstalledBinaryTest(unittest.TestCase):
    def setUp(self):
        # The sanctioned fixture, not a hand-rolled config.
        #
        # This wrote its own TOML with `collection_name` and
        # `accept_invalid_certs` under [profiles.default] — neither is a profile
        # key, so the default collection it thought it was setting never took
        # effect. Loading the shared fixture is itself part of what the suite
        # proves; see protocols/testing-and-release-gate.md rule 1.
        self.env = {
            **os.environ,
            "VECDB_CONFIG": FIXTURE,
            "VECDB_ALLOW_LOCAL_FS": "true",
        }
        self.server_bin = os.path.join(INSTALL_ROOT, "vecdb-server")
        self.process = None

    def tearDown(self):
        if self.process:
            self.process.terminate()
            try:
                self.process.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()

    def _rpc(self, method, params=None):
        req = {"jsonrpc": "2.0", "method": method, "id": 1}
        if params:
            req["params"] = params
        self.process.stdin.write(json.dumps(req) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise AssertionError(f"Server died: {self._stderr()}")
        return json.loads(line)

    def test_installed_binaries_match_this_tree(self):
        """The whole point: what is installed is what was just tested."""
        want = expected_revision()
        if want is None:
            self.skipTest("not a git checkout — no revision to compare against")

        problems = []
        for name in ("vecdb", "vecdb-server"):
            path = os.path.join(INSTALL_ROOT, name)
            if not os.path.exists(path):
                problems.append(f"{name}: not installed at {path}")
                continue
            got = revision_of(path)
            if got != want:
                problems.append(
                    f"{name}: installed build reports git:{got or 'no revision'}, "
                    f"this tree is git:{want}"
                )

        if problems:
            self.fail(
                "The installed binaries are not what this tree builds:\n  "
                + "\n  ".join(problems)
                + "\n\n"
                "This is the binary your shell, your cron and your MCP client run.\n"
                "A green suite says nothing about it until it matches.\n\n"
                "  make install                 # standard build\n"
                "  make install-cuda-dynamic    # BYO ONNX Runtime (docs/GPU_LEGACY.md)\n"
            )
        print(f"installed vecdb, vecdb-server both report git:{want} ✅")

    def test_installed_server_answers_mcp(self):
        """Still worth asserting — a matching revision that cannot start is no use."""
        if not os.path.exists(self.server_bin):
            self.skipTest(f"{self.server_bin} not installed")

        self.process = subprocess.Popen(
            [self.server_bin, "--stdio", "--allow-local-fs"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=self.env,
        )
        # Drain stderr continuously so the server can never block writing to
        # a full stderr pipe while this test blocks reading stdout.
        # See tests/lib_stdio.py for the deadlock this prevents.
        self._stderr = drain_stderr(self.process)
        time.sleep(1)

        self._rpc("initialize")
        res = self._rpc("resources/list")
        self.assertNotIn("error", res, f"resources/list failed: {res.get('error')}")

        resources = res["result"]["resources"]
        manual = next((r for r in resources if r["uri"] == "vecdb://manual"), None)
        self.assertIsNotNone(manual, "vecdb://manual missing from the installed binary")
        print(f"installed server answers MCP, {len(resources)} resources ✅")


if __name__ == "__main__":
    unittest.main()
