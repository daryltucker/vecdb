# vecdb & vecq Example Cookbook

This guide serves as the definitive cookbook for using the `vecdb` suite (Database & Query Tool).

---

## 🚀 Part 1: "The Detective" (vecq)
**vecq** is `jq` for source code. Use it for precise, structural queries that regex cannot handle.

### 🔍 1.1 Structural Grep (Recursive Search)
Replace `grep -r` with structure-aware queries. `vecq` automatically recurses into supported files (`-R`).

> **💡 Agent Tip**: For a deep-dive into advanced recipes (Complexity Analysis, API Auditing, etc.), see:
> [vecq & vecdb Recipe Cookbook](file:///home/daryl/Projects/NRG/vecdb-mcp/docs/vecq/EXAMPLES.md)

**Find all public functions in `src/`:**
```bash
vecq -R src/ -q '(.functions // [])[] | select(.visibility == "pub") | .name' --grep-format
```
*Why better than grep?* It ignores "pub" inside comments or strings.

**Find all functions that return a `Result`:**
```bash
vecq -R src/ -q '.functions[] | select(.return_type | contains("Result"))' --grep-format
```

### 📦 1.2 Code Block Extraction (The "Agent Win")
Extract clean, executable code from Markdown documentation. Critical for agents reading implementation guides.

**Extract all bash commands from a README:**
```bash
vecq README.md -q '.code_blocks[] | select(.attributes.language == "bash") | .content' -r
```
*Note: The `-r` (raw output) flag is essential to get clean text without quotes.*

### 📊 1.3 Codebase Analysis
**Count total functions per file:**
```bash
vecq -R src/ -q 'file + ": " + (.functions | length | tostring)' -r
```

**List all imported modules:**
```bash
vecq -R src/ -q '.imports[]' --grep-format
```

### 🛡️ 1.4 Best Practices (Ready-to-Use Queries)
**The API Extractor (Public Interface):**
```bash
vecq -R src/ -q '.functions[] | select(.attributes.visibility == "pub") | {name, signature: .attributes.signature}'
```

**The Safety Auditor (Find `unsafe` usage):**
```bash
vecq -R src/ -q '.functions[] | select(.content | contains("unsafe"))'
```

**The Tech Debt Hunter (Find `todo!` macros):**
```bash
vecq -R src/ -q '.functions[] | select(.content | contains("todo!"))'
```

**Find functions that lack documentation (missing doc attributes):**
```bash
vecq -R src/ -q '.functions[] | select(.attributes.docs == null) | .name' --grep-format
```

**Normalize a raw log to a canonical schema:**
```bash

cat access.log | vecq -q 'openwebui_to_chat | .[] | select(.role == "user")'
```

---

## 📚 Part 2: "The Librarian" (vecdb)
**vecdb** is your semantic memory. Use it to find *concepts* and *meanings*.

### 📥 2.1 Ingestion
**Ingest a documentation folder:**
```bash
vecdb ingest ./docs --collection docs --chunk-size 512
```

**Ingest a code repository (respecting .gitignore):**
```bash
vecdb ingest ./src --collection my_code --respect-gitignore
```

**Ingest from a pipe (single file/stream):**
```bash
cat Important_Note.txt | vecdb ingest - --collection notes
```

### 🧠 2.2 Semantic Search
**Basic Conceptual Search:**
```bash
vecdb search "How do I configure profiles?"
```

**Search specific collection:**
```bash
vecdb search "memory safety patterns" --collection rust_code
```

**Machine-Readable Output (for scripts/agents):**
```bash
vecdb search "error handling" --json | jq .
```

### 🛠️ 2.3 Management
**Check status & connectivity:**
```bash
vecdb status
```

**List available collections:**
```bash
vecdb list
```

---

## 🤖 Part 3: MCP Agent Usage
Examples of how an AI Agent utilizes these tools via the Model Context Protocol.

### 3.1 Learning a New Codebase (`ingest_path`)
*Agent*: "I need to understand the new module at `/src/auth`."
```json
{
  "name": "ingest_path",
  "arguments": {
    "path": "/home/user/projects/app/src/auth",
    "collection": "auth_module"
  }
}
```

### 3.2 Solving a Bug (`search_vectors`)
*Agent*: "How does the authentication middleware handle timeouts?"
```json
{
  "name": "search_vectors",
  "arguments": {
    "query": "authentication middleware timeout handling",
    "collection": "auth_module"
  }
}
```

### 3.3 Surgical Extraction (`code_query`)
*Agent*: "I need the `User` struct definition to mock it."
```json
{
  "name": "code_query",
  "arguments": {
    "path": "/home/user/projects/app/src/auth/types.rs",
    "query": ".structs[] | select(.name == \"User\")"
  }
}
```
*Note: `code_query` is powered by `vecq`.*


## 🎨 Part 4: Advanced Algorithmic Demos
See `vecq` analyze complex simulation logic and generate documentation from it.

### 4.1 Wave Function Collapse (Entropy Minimization)
*   **Source**: `demo/algorithms/wfc.rs`
*   **Generated Manual**: `demo/algorithms/wfc_doc.md`
*   **Concept**: Extracting structural logic from a chaos theory algorithm.

**Generate it yourself:**
```bash
vecq doc demo/algorithms/wfc.rs > demo/algorithms/wfc_doc_gen.md
```

### 4.2 Boids Flocking (Vector Math)
*   **Source**: `demo/algorithms/boids.rs`
*   **Generated Manual**: `demo/algorithms/boids_doc.md`
*   **Concept**: Documenting physics rules (`rule_cohesion`, `rule_separation`) as API contracts.
*   **Visual**: Run `rustc demo/algorithms/boids.rs && ./boids` to see them fly!

**Generate it yourself:**
```bash
vecq doc demo/algorithms/boids.rs > demo/algorithms/boids_doc_gen.md
```

Verdict: Yes, code-as-JSON allows us to query structure (Logic, Safety, Visibility) rather than just text (Strings). It lets us ask "Show me all public functions that are unsafe" in one line, which grep struggles with (context issues).