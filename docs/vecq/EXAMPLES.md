# vecq Examples Cookbook

This document provides a cookbook of common queries and recipes for `vecq`.

## 🚀 Key Patterns

### 1. Structural Grep (Recursive)
The "Happy Path" for finding code. Uses `-R` to recurse and `--grep-format` for editor compatibility.

**Find public functions:**
```bash
vecq -R src/ -q '.functions[] | select(.visibility == "pub") | .name' --grep-format
```

**Find "TODO" items:**
```bash
vecq -R src/ -q '.todos[]' --grep-format
```

### 2. Code Block Extraction
Extract executable code from Markdown files. Clean, raw output.

**Extract bash commands:**
```bash
vecq README.md -q '.code_blocks[] | select(.attributes.language == "bash") | .content' -r
```

---

## 🦀 Rust Examples

### List all struct names
```bash
vecq src/lib.rs '.structs[] | .name'
```

### Find functions returning Result
```bash
vecq -R src/ '.functions[] | select(.return_type | contains("Result")) | .name'
```

### List imports
```bash
vecq -R src/ '.imports[]' --grep-format
```

---

## 🐍 Python Examples

### List all classes
```bash
vecq app.py '.classes[] | .name'
```

### Find decorated functions
```bash
vecq app.py '.functions[] | select(.decorators | length > 0) | .name'
```

---

## 📝 Markdown Examples

### List document headers
```bash
vecq README.md '.headers[] | .title'
```

### Extract Rust code blocks
```bash
vecq README.md -q '.code_blocks[] | select(.attributes.language == "rust") | .content' -r
```

---

## ⚡ Terminal Workflows

### Syntax Highlighting with `vecdb`
Pipe `vecdb search` results into `vecq` for coloring:
```bash
vecdb search "authentication" | vecq syntax -l md
```

### Count lines of code (function bodies)
```bash
vecq -R src/ -q '.functions[] | .line_end - .line_start' | paste -sd+ - | bc
```
