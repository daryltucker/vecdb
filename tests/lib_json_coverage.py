"""Library for the T1.19 gate (`tier1_json_chunk_coverage.py`): does every piece
of a JSON file come out of the chunker exactly once?

Token comparison cannot answer this for JSON: chunks may re-serialise values,
so whitespace and quoting differ from the source. This compares STRUCTURE:

  scalars    every string/number/bool/null in the source, as a value
  keys       every object key (a key is covered by a parsed member, or by a
             label line naming it — a descended container's key lives there)
  empties    every `{}` / `[]` (they have no scalars, so would vanish unseen)

Split parts are reassembled (consecutive `split_part` 0..n of one chunk)
before parsing. Each chunk body is cut on blank lines; a piece is data if it
parses as a JSON value or as a `"key": value` member, otherwise it is a label
line (key path / identity written by the packer) and is reported if it is
neither.

The chunks come from the built `vecdb ingest --dry-run --show-chunks --json`
against the TEST config — nothing is embedded or written.
"""

import collections
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from paths import bin_path  # noqa: E402

TESTS = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(TESTS)
CONFIG = os.path.join(TESTS, "fixtures", "config.toml")
# The fixture profile whose model can hold large chunks; see config.toml.
PROFILE = "tier1_large_chunks"


def dump(paths, target=1200, pack=2048, extra=()):
    """Chunks as dicts, in emission order, exactly as they would be embedded."""
    env = {**os.environ, "VECDB_CONFIG": CONFIG, "VECDB_ALLOW_NO_GPU": "1"}
    proc = subprocess.run(
        [bin_path("vecdb"), "--json", "--profile", PROFILE, "ingest",
         "-c", "test_chunk_dump", "--dry-run", "--show-chunks",
         "--target-chunk-size", str(target), "--pack-target-bytes", str(pack),
         *extra, *paths],
        capture_output=True, text=True, env=env)
    if proc.returncode != 0:
        raise RuntimeError(f"vecdb exited {proc.returncode}:\n{proc.stderr[-2000:]}")
    return [json.loads(l) for l in proc.stdout.splitlines() if l.startswith("{")]


def body(chunk):
    """Chunk content without the label the packer prefixed."""
    label = chunk.get("context_label")
    content = chunk["content"]
    if label and content.startswith(label + "\n\n"):
        return content[len(label) + 2:]
    return content


def strip_jsonc(text):
    """(JSON text, [comments]): `//` and `/* */` comments removed, and the
    trailing commas JSONC allows — string-aware, so `"http://x"` survives."""
    out, comments, i, n = [], [], 0, len(text)
    while i < n:
        c = text[i]
        if c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i:j + 1])
            i = j + 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            comments.append(text[i:j])
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            comments.append(text[i:j])
            i = j
        else:
            out.append(c)
            i += 1
    clean = "".join(out)
    # Trailing commas: `,` followed only by whitespace before `}` / `]`.
    result, i = [], 0
    while i < len(clean):
        c = clean[i]
        if c == '"':
            j = i + 1
            while j < len(clean) and clean[j] != '"':
                j += 2 if clean[j] == "\\" else 1
            result.append(clean[i:j + 1])
            i = j + 1
            continue
        if c == ",":
            k = i + 1
            while k < len(clean) and clean[k].isspace():
                k += 1
            if k < len(clean) and clean[k] in "}]":
                i += 1
                continue
        result.append(c)
        i += 1
    return "".join(result), comments


def roots_of(text):
    """Top-level values: one document, or JSONL / concatenated roots."""
    dec = json.JSONDecoder()
    roots, i = [], 0
    while True:
        while i < len(text) and text[i].isspace():
            i += 1
        if i >= len(text):
            return roots
        value, i = dec.raw_decode(text, i)
        roots.append(value)


def canon(v):
    return json.dumps(v, sort_keys=True, ensure_ascii=False)


def walk(v, scalars, keys, empties):
    if isinstance(v, dict):
        if not v:
            empties["{}"] += 1
        for k, child in v.items():
            keys[k] += 1
            walk(child, scalars, keys, empties)
    elif isinstance(v, list):
        if not v:
            empties["[]"] += 1
        for child in v:
            walk(child, scalars, keys, empties)
    else:
        scalars[canon(v)] += 1


def one_item(piece):
    """(value, member_key) if `piece` is exactly ONE JSON value or ONE
    `"key": value` member (comments and trailing commas allowed), else None.

    Exactly one, on purpose: the packer separates members with blank lines, so
    a data piece never holds two items — while a label line can look like JSON
    (`[1] {"id": 7}` is a trail plus an identity, two items)."""
    clean, _ = strip_jsonc(piece)
    clean = clean.strip()
    if not clean:
        return None
    for wrapped, member in ((clean, False), ("{" + clean + "}", True)):
        try:
            value = json.loads(wrapped)
        except ValueError:
            continue
        if not member:
            return value, None
        if len(value) == 1:
            (k, v), = value.items()
            return v, k
    return None


def is_comment_only(piece):
    clean, comments = strip_jsonc(piece)
    return bool(comments) and not clean.strip()


def reassemble(chunks):
    """Bodies with split parts joined back into the chunk they came from."""
    out = []
    for c in chunks:
        part = c.get("split_part")
        b = body(c)
        if part and out:
            out[-1] = (out[-1][0] + b, out[-1][1])
        else:
            out.append((b, c.get("context_label") or ""))
    return out


def classify(text):
    """Split a chunk body into data items and label lines.

    Pieces are blank-line separated. A piece that is not one item may be a
    pretty-printed value whose source had blank lines inside it, so it is
    joined with the following pieces until it parses; failing that, a
    single-line piece is a label."""
    pieces = text.split("\n\n")
    data, labels, i = [], [], 0
    while i < len(pieces):
        piece = pieces[i]
        if not piece.strip():
            i += 1
            continue
        parsed = one_item(piece)
        if parsed is not None and "\n" not in piece.strip() and _looks_like_trail(piece):
            parsed = None
        if parsed is not None:
            data.append(parsed)
            i += 1
            continue
        joined = None
        for j in range(i + 1, min(i + 40, len(pieces))):
            candidate = "\n\n".join(pieces[i:j + 1])
            got = one_item(candidate)
            if got is not None:
                joined = (got, j)
                break
        if joined and "\n" in piece:
            data.append(joined[0])
            i = joined[1] + 1
            continue
        labels.append(piece)
        i += 1
    return data, labels


def _looks_like_trail(piece):
    """`[3]` alone parses as JSON but, on its own line, is an array-item trail."""
    s = piece.strip()
    return s.startswith("[") and s.endswith("]") and s[1:-1].isdigit()


def measure(path, **kw):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    clean_src, src_comments = strip_jsonc(text)
    src_s, src_k, src_e = collections.Counter(), collections.Counter(), collections.Counter()
    for r in roots_of(clean_src):
        walk(r, src_s, src_k, src_e)

    chunks = dump([path], extra=("--bypass-vectorignore",), **kw)
    out_s, out_k, out_e = collections.Counter(), collections.Counter(), collections.Counter()
    out_comments = collections.Counter()
    labels = []
    for b, label in reassemble(chunks):
        labels.append(label)
        _, comments = strip_jsonc(b)
        out_comments.update(c.strip() for c in comments)
        data, label_lines = classify(b)
        labels.extend(label_lines)
        for value, member_key in data:
            if member_key is not None:
                out_k[member_key] += 1
            walk(value, out_s, out_k, out_e)

    label_text = "\n".join(labels)
    # The packer writes a record's identity (its short scalar members, as a
    # JSON object) into the label of EVERY chunk cut from it, and does not emit
    # those members again as data. They are therefore present when they appear
    # in a label at least once — never counted per repetition, which is by
    # design, not duplication.
    label_scalars, label_keys = collections.Counter(), collections.Counter()
    for label in labels:
        for piece in label.split("\n\n"):
            # Label lines are `{identity}`, `trail`, or `trail {identity}`.
            brace = piece.find("{")
            parsed = one_item(piece[brace:]) if brace >= 0 else None
            if parsed is not None and isinstance(parsed[0], dict):
                walk(parsed[0], label_scalars, label_keys, collections.Counter())
    out_s = out_s + collections.Counter(
        {s: n for s, n in (src_s - out_s).items() if s in label_scalars})
    out_k = out_k + collections.Counter(
        {k: n for k, n in (src_k - out_k).items() if k in label_keys})
    # A key absent from the data but named in a label is a descended container.
    missing_k = collections.Counter({
        k: n for k, n in (src_k - out_k).items()
        if k not in label_text
    })
    src_c = collections.Counter(c.strip() for c in src_comments)
    return {
        "chunks": len(chunks),
        "scalars": sum(src_s.values()),
        "missing_scalars": src_s - out_s,
        "extra_scalars": out_s - src_s,
        "missing_keys": missing_k,
        "extra_keys": out_k - src_k,
        "missing_empty": src_e - out_e,
        "extra_empty": out_e - src_e,
        "missing_comments": src_c - out_comments,
        "extra_comments": out_comments - src_c,
        # Label lines, shown so a human can confirm they are labels and not
        # mangled data.
        "label_lines": [l for l in labels if l],
    }
