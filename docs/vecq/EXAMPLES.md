# vecq Examples Cookbook

`vecq` is a versatile tool for structural querying. Below are common patterns to help you navigate and transform your documents.

---

## 🔍 Search Recipes

### Find text in Headers or Paragraphs
This query searches both Markdown headers (titles) and paragraphs for a specific term. Use this when you want to find content within specific structural elements rather than just a raw string match.

```bash
vecq -R . -q '(.headers[] | select(.name | contains("pattern"))), (.paragraph[] | select(.content | contains("pattern")))' --grep-format
```

### Search All Content Elements
To search across all document elements (code blocks, text blocks, headers, etc.):

```bash
vecq -R . -q '.elements[] | select(.content | contains("pattern"))' --grep-format
```

### Filter by File Type
Process only specific file types (e.g., Markdown) within a directory:

```bash
find . -name "*.md" | xargs vecq -q '.headers[]' --grep-format
```

---

## �️ Extraction Recipes

### Extract all functions from a project
```bash
vecq -R src/ '.functions[]'
```

### List all TODOs in Bash scripts
```bash
vecq -R scripts/ -q '.comments[] | select(.content | contains("TODO"))' --grep-format
```

### Extract Python class names
```bash
vecq app/ '.classes[] | .name'
```

---

## 🎨 Visualization and Piping

### Syntax highlight search results
Pipe `vecdb search` results into `vecq` for colorized structural analysis:
```bash
vecdb search "authentication" | vecq syntax -l md
```

### Count lines of code (function bodies)
```bash
vecq -R src/ -q '.functions[] | .line_end - .line_start' | paste -sd+ - | bc
```