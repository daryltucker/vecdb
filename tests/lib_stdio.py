"""Talking to `vecdb-server --stdio` without deadlocking it.

THE DEADLOCK
    The stdio MCP transport uses two pipes: protocol responses on the child's
    stdout, logs and progress on its stderr. A test that does the obvious thing —

        proc = Popen(..., stdout=PIPE, stderr=PIPE)
        proc.stdin.write(request); proc.stdin.flush()
        line = proc.stdout.readline()          # <-- blocks here

    is reading only ONE of those pipes. The other has a fixed kernel buffer
    (64 KiB on Linux). Once the child has written that much to stderr, its next
    write(2) blocks; it therefore never finishes composing the reply on stdout;
    and the test blocks forever reading a stdout that will never produce a line.
    Neither side is broken and neither side times out.

    Measured 2026-257: `tier4_realistic_ingest.py` ingesting Lua 5.4.6 wedged
    after 1115 points with every one of the server's 30 threads asleep — one in
    `anon_pipe_write`, the test in `anon_pipe_read`. It had been running 40
    minutes with no timeout and would have run indefinitely.

    The trigger was a per-chunk WARN, since demoted to DEBUG
    (`ingestion/pipeline.rs`). That removes THIS instance. It does not remove the
    hazard: any sufficiently chatty ingest reaches 64 KiB eventually, and the
    fix belongs on both sides.

THE FIX
    Drain stderr continuously on a separate thread, so the child can never block
    writing to it. `drain_stderr(proc)` does that and hands back an accessor for
    everything captured — which is also strictly better for diagnostics, because
    the old shape only read stderr in the failure path and discarded it
    otherwise.

    Use `subprocess.communicate()` instead where a one-shot call fits; it already
    drains both pipes concurrently. This helper is for the interactive,
    request/response shape, where communicate() cannot be used.

NOTE FOR REAL CLIENTS
    This is a test helper, but the constraint is not test-specific: any MCP host
    that pipes vecdb-server's stderr must drain it. Keeping server chatter low
    is what keeps that from being a sharp edge.
"""

import threading


def drain_stderr(proc):
    """Continuously read `proc.stderr` on a daemon thread.

    Returns a zero-argument callable giving everything captured so far, as one
    string. Safe to call at any time, including after the process exits.

    The thread is a daemon so a wedged child can never keep the interpreter
    alive; it ends on EOF, which happens when the child closes stderr or exits.
    """
    chunks = []
    lock = threading.Lock()

    def pump():
        # Iterating the file object yields lines until EOF. Errors are swallowed
        # deliberately: this thread exists to keep the pipe empty, and a failure
        # to read must never be what fails a test — the test's own assertions
        # are what report the problem.
        try:
            for line in proc.stderr:
                with lock:
                    chunks.append(line)
        except (ValueError, OSError):
            pass

    t = threading.Thread(target=pump, name="stderr-drain", daemon=True)
    t.start()

    def captured():
        with lock:
            return "".join(chunks)

    # The thread is returned alongside so a caller that wants a clean shutdown
    # can join it after the process exits. Most callers ignore it.
    captured.thread = t
    return captured
