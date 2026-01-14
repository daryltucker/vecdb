# vecq & vecdb Recipe Cookbook: The Agent's Guide

This document is a collection of high-value recipes for AI Agents using `vecq` (Structural Search) and `vecdb` (Semantic Search).

---

## 🛠️ Section 1: Structural Analysis Recipes (`vecq`)

These recipes help you understand *how* the code is built.

### 1.1 The "API Surface Auditor"
**Goal**: Quickly understand the public interface of a module without reading the implementation details.
**Why**: As an agent, you often need to know "what can I call?" before writing code.

```bash
vecq -R src/ -q '(.functions // [])[] | select(.visibility == "pub") | {name, signature}' --compact
```
*   **Tip**: Use `(.functions // [])` to safely handle files that might lack functions.

### 1.2 The "Test Integrity Check"
**Goal**: Find test functions that might be empty or missing assertions (fragile tests).
**Why**: Before fixing a bug, ensure the tests actually test something.

```bash
vecq -R tests/ -q '(.functions // [])[] | select(.name | contains("test")) | select(.content | contains("assert") | not) | .name' --grep-format
```

### 1.3 The "Complexity Hunter"
**Goal**: Find functions that are too long or deeply nested.
**Why**: These are prime candidates for bugs or refactoring.

```bash
vecq -R src/ -q '(.functions // [])[] | select((.line_end - .line_start) > 50) | {name, lines: (.line_end - .line_start)}' --compact
```

### 1.4 The "Dependency Mapper"
**Goal**: Find which files import a specific module.
**Why**: Understanding the blast radius of a change.

```bash
vecq -R src/ -q '(.imports // [])[] | select(.path | contains("my_module")) | .path' --grep-format
```

### 1.5 The "Legacy Archaeologist" (Advanced)
**Goal**: Find a class with a specific name AND a specific method pattern (e.g., verifying virtual destructors in C++).
**Scenario**: You suspect `SmartHandle` is leaking memory because it lacks a virtual destructor.

```bash
vecq include/ -q '.structs[] | select(.name == "SmartHandle") | .methods[] | select(.name == "~SmartHandle")'
```

### 🕵️ 1.6 Regex-Powered Structural Queries
**Goal**: // TODO
**Scenario**: // TODO

**Find all TODOs in comments with a specific ticket ID:**
```bash
vecq -R src/ -q '.comments[] | select(test("TODO\\[VEC-\\d+\\]"))' --grep-format
```

**Extract and normalize JSDoc-style parameters from Python docstrings:**
```bash
vecq src/main.py -q '.functions[] | .attributes.docs | capture("@param {(?<type>\\w+)} (?<name>\\w+)")'
```

### 1.7 Pioritizing Tasks (Eager Beaver)
**Goal**: Work efficiently, with the starting with the highest priority tasks
**Scenario**: Selecting a new task, filter out lower-priority items

```bash
vecq -t text src/ -q 'todo_to_task | select(.priority == "high")'
```

---

## 🧠 Section 2: Semantic Analysis Recipes (`vecdb`)

These recipes help you understand *what* the code does (concepts).

### 2.1 The "Concept Search"
**Goal**: Find code related to a concept, even if the variable names are obscure.
**Why**: `grep` fails on "memory management" if the code uses `alloc` or `malloc`. `vecdb` succeeds.

```bash
vecdb search "custom memory allocator pool mechanism" --collection src --json
```

### 2.2 The "Codebase Ingestion Pattern"
**Goal**: Quickly memorize a new repository to ask questions about it.

1.  **Ingest**: `vecdb ingest . --collection project_x --respect-gitignore`
2.  **Verify**: `vecdb list` (Ensure vectors are created)
3.  **Query**: `vecdb search "main entry point logic" --collection project_x`

---

## 🎨 Visualization and Piping

### Syntax highlight search results
Pipe `vecdb search` results into `vecq` for colorized structural analysis:
```bash
vecdb search "authentication" | vecq syntax -l md
```

### Count lines of code (function bodies)
```bash
vecq -R src/ -q '(.functions // [])[] | .line_end - .line_start' | paste -sd+ - | bc
```



## 📄 Section 3: Markdown Recipes (The "Documentary" Series)
**Note**: Some recipes assume upcoming parser enhancements (Task Lists, Emphasis, HTML).

### 3.1 The "Link Auditor"
**Goal**: extracting every link with its visible text to check for dead URLs or poor labeling.
```bash
vecq README.md -q '.links[] | "\(.name) -> \(.content)"' -r
# Output: "Build Instructions -> docs/BUILDING.md"
```

### 3.2 The "Table of Contents" Generator
**Goal**: Visualizing the document hierarchy (headers).
```bash
vecq ARCHITECTURE.md -q '.headers[] | "\("#" * .level) \(.content)"' -r
# Output:
# # System Overview
# ## Components
# ### Ingestion
```

### 3.3 The "Unfinished Business" (Task Finder)
**Goal**: Finding all unchecked task list items.
```bash
vecq TODO.md -q '.list_items[] | select(.task == true and .checked == false) | .content'
```

### 3.4 The "Accessibility Audit"
**Goal**: Finding images missing Alt Text.
```bash
vecq blog_post.md -q '.images[] | select(.name == "") | "Missing Alt Text: \(.content)"'
```

### 3.5 The "Code Extractor"
**Goal**: Pulling out all JSON configuration snippets from documentation.
```bash
vecq CONFIG.md -q '.code_blocks[] | select(.attributes.language == "json") | .content' -r
```

### 3.6 The "Raw HTML Hunter"
**Goal**: Finding places where raw HTML was used instead of Markdown.
```bash
vecq README.md -q '.html_blocks[] | .content'
```

### 3.7 The "Emphasis Miner"
**Goal**: Extracting bolded terms to build a glossary key.
```bash
vecq glossary.md -q '.elements[] | select(.type == "Strong") | .content'
```

### 3.8 The "Footnote Checker"
**Goal**: Listing all footnote definitions to ensure they match references.
```bash
vecq paper.md -q '.footnotes[] | "\(.name): \(.content)"'
```

### 3.9 The "Citation Catcher"
**Goal**: Extracting blockquotes to review external references.
```bash
vecq design.md -q '.blockquotes[] | .content'
```

### 3.10 The "Relative Path Validator"
**Goal**: Finding relative links that might break when moved.
```bash
vecq doc.md -q '.links[] | select(.content | startswith("http") | not) | .content'
```

---

## Misc

```bash
# Show all Links in README.md
vecq README.md -q '.links[] | "\(.name)\t\(.content)"' -r | column -t -s $'\t'
```

```bash
vecq docs/vecq/EXAMPLES.md -q '[(.emphasis // []), (.strong // []), (.strikethrough // []), (.list_items // []), (.footnotes // [])] | flatten | .[] | {type: .type, content: .content}'
```