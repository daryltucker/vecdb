# Parsing Guidelines: The Code Skeleton

> **Goal**: Enable a "tree view" for source code, treating code entities (functions, classes) with the same hierarchical respect as Markdown headers.

## core Philosophy

We parse code not just to index it, but to **understand its shape**. 
Just as a Markdown file has a skeleton defined by its headers, every source file has a skeleton defined by its symbols.

- **Markdown**: `H1` -> `H2` -> `H3`
- **Python**: `Class` -> `Method` -> `Inner Function`
- **Rust**: `Module` -> `Impl` -> `Function`

All code parsers **MUST** extracting this hierarchical structure to support the "Universal Tree View".

## The Universal Tree View Requirements

All parsers must produce a `ParsedDocument` that satisfies these structural requirements:

### 1. Explicit Hierarchy (Nesting)
Elements must be nested in the `children` vector of their parent.
- **Root Level**: Top-level functions, classes, and constants must be in the `elements` root array.
- **Nested Level**: Methods inside a class must be in the `children` array of the Class element.
- **Deep Nesting**: Inner functions or closures should be children of their parent function.

**Bad (Flat)**:
```json
[
  {"type": "class", "name": "MyClass"},
  {"type": "method", "name": "my_method"} // ❌ Lost relationship
]
```

**Good (Nested)**:
```json
[
  {
    "type": "class", 
    "name": "MyClass",
    "children": [
      {"type": "method", "name": "my_method"} // ✅ explicit heirarchy
    ]
  }
]
```

### 2. The "Skeleton" Entities
Every language parser must identify and extract these key "Structural Entities":

| Concept | Markdown | Rust | Python | C/C++ | Go |
|---------|----------|------|--------|-------|----|
| **Container** | Header (H1-H6) | Module / Struct / Impl | Class | Namespace / Class | Package / Struct |
| **Unit** | Paragraph / Block | Function | Function / Method | Function | Function |
| **Interface** | Link | Trait | Abstract Base Class | Abstract Class / Header | Interface |

### 3. Line-Precise Spans
Every element must strictly define its `line_start` and `line_end`.
- The `line_span` of a parent **MUST** fully contain the `line_span` of its children.
- This allows us to "fold" code in UI or CLI by hiding lines between `start` and `end`.

## Usage Scenarios

### The `tree` Command for Code
Users should be able to visualize any file's structure:

```bash
$ vecq tree src/main.rs
└── [mod] main
    ├── [struct] AppState
    │   └── [field] db_url
    └── [fn] main
        └── [fn] init_db (inner)
```

### Context Windowing
When feeding an LLM, we can pass just the **Skeleton** (headers/definitions) without the **Organ Meat** (implementation bodies) to save tokens while providing high-level map.

## Implementation Checklist for Parsers

When implementing `Parser` trait for a new language:

1. [ ] **Identify Scope**: What defines a "block" or "scope" in this language? (Braces `{}`, Indentation, `end` keywords).
2. [ ] **Capture Children**: When parsing a scope, recursively capture child elements.
3. [ ] **Assign Parent**: Ensure child elements are pushed into the parent's `children` list.
4. [ ] **Verify Spans**: Ensure parent line range covers all children.
