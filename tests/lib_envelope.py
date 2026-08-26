"""Shared helpers for reading the vecdb search response envelope.

Both `vecdb search --json` and the MCP `search_vectors` tool return an object,
not a bare list:

    {collection, query, limit, min_score, applied_filters, result_count, results[]}

The envelope exists so a caller can tell "no matches" apart from "matched, then
truncated at the limit" — see `result_count == limit`.

Every consumer must go through `search_results()`. Calling `len()` on the
envelope counts its KEYS, which is a positive number no matter how many
documents matched, so `assert len(payload) > 0` silently passes on an empty
result set. That mistake was live in three tests at once; centralising the
access is what stops it recurring.
"""

REQUIRED_FIELDS = ("collection", "query", "result_count", "results")


def search_results(payload, *, context=""):
    """Validate a search envelope and return its result list.

    Raises AssertionError with an explanatory message if the shape is wrong,
    so a contract change surfaces as a named failure rather than a KeyError.
    """
    where = f" ({context})" if context else ""

    assert isinstance(payload, dict), (
        f"expected a search envelope object{where}, got {type(payload).__name__}. "
        "A bare list means something is still emitting the pre-envelope format."
    )

    missing = [f for f in REQUIRED_FIELDS if f not in payload]
    assert not missing, f"search envelope missing {missing}{where}: {sorted(payload)}"

    results = payload["results"]
    assert isinstance(results, list), (
        f"'results' must be a list{where}, got {type(results).__name__}"
    )

    assert payload["result_count"] == len(results), (
        f"result_count={payload['result_count']} but {len(results)} results "
        f"present{where} — the count and the payload disagree"
    )

    return results


def result_path(result):
    """Path of a single search result. SearchResult uses 'metadata', not 'payload'."""
    return result.get("metadata", {}).get("path", "unknown")
