# The vecdb & vecq Cookbook
*Recipes for Semantic Discovery and Structural Mastery*

This guide provides high-level narrative "cookbooks" for the most common workflows using `vecdb` and `vecq`.

---

## 🍳 Recipe 1: "The New Hire" (Onboarding to a Codebase)
**Goal**: You just joined a massive project. You don't know where `main` is, or how the auth works.

1.  **Ingest Everything**:
    ```bash
    vecdb ingest . --collection monolith --respect-gitignore
    ```
2.  **Ask Questions**:
    ```bash
    vecdb search "Where is the core initialization logic?"
    vecdb search "How do we handle JWT tokens?"
    ```
3.  **Audit Structure**:
    Once `vecdb` points you to `src/auth/jwt.rs`, use `vecq` to see its public API:
    ```bash
    vecq src/auth/jwt.rs -q '.functions[] | select(.visibility == "pub")'
    ```

## 🍳 Recipe 2: "The Surgical Fix" (Time Travel & Comparison)
**Goal**: A bug appeared in `v1.2.0`. You want to see how `parser.rs` looked in `v1.1.0`.

1.  **Ingest the Past**:
    ```bash
    vecdb ingest-history . --git-ref v1.1.0 --collection legacy-parser
    ```
2.  **Compare Logic**:
    ```bash
    # Search the legacy version for the buggy concept
    vecdb search "error handling in parser" --collection legacy-parser
    ```
3.  **Extract the old code**:
    ```bash
    vecq parser.rs -q '.functions[] | select(.name == "handle_error")' -r
    ```

## 🍳 Recipe 3: "The Speed Demon" (Optimization)
**Goal**: Your collection is huge and search is sluggish or uses too much RAM.

1.  **Quantize**:
    ```bash
    vecdb config set-quantization monolith scalar
    vecdb optimize monolith
    ```
2.  **Monitor**:
    ```bash
    vecdb status
    ```
    *Look for "Active Remote Tasks (Qdrant)" to see the quantization progress.*

---

## 📚 Advanced Deep Dives
For high-density technical recipes (Complexity analysis, Security auditing, etc.), see:
*   [vecq Technical Recipes](file:///home/daryl/Projects/NRG/vecdb-mcp/docs/vecq/EXAMPLES.md)
*   [Agent Manual](file:///home/daryl/Projects/NRG/vecdb-mcp/docs/AGENT_MANUAL.md)