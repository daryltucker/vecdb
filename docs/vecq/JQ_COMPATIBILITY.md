# JQ Compatibility Guide (jaq vs jq)

This document tracks identified differences between `jaq` (the Rust implementation used by `vecq`) and standard C-based `jq`. 

## Philosophy: Parse, Don't Guess
We treat these differences as part of our technical debt and architectural constraints. Every difference must be evaluated: does it make us stronger (performance/safety) or is it an inconvenience?

---

## ✅ Verified Supported (High Parity)
*   **Regex**: `test`, `match`, `sub`, `gsub`
*   **Hierarchical**: `walk(...)`, `to_entries`, `keys`, `type`, `sort_by`
*   **Conversions**: `fromdateiso8601`, `tonumber`, `tostring`, `@json`
*   **Functional**: `map`, `select`, `reduce`, `foreach`
*   **Safety**: `try (...) catch ...`, `empty`
*   **Performance**: Recursive function calls (tail-call optimized)

---

## 🛑 Critical Differences

### 1. Date Parsing (`strptime`)
*   **Status**: ❌ **UNSUPPORTED** in `jaq` (core).
*   **Impact**: HIGH. Prevents parsing non-standard timestamp formats (e.g. Syslog, Nginx defaults).
*   **Evaluation**: **Inconvenient**. We must normalize timestamps to ISO8601 in the Rust parser level or use string slicing if the format is fixed.
*   **Workaround**: `fromdateiso8601` is supported. Use it for standard formats.

### 2. Variable Binding Syntax (`let`)
*   **Status**: ❌ **UNSUPPORTED**. `jaq` does not recognize the `let ($v = exp) | ...` syntax.
*   **Impact**: MEDIUM. Breaks many legacy `jq` scripts found online.
*   **Evaluation**: **Stronger**. Forces the cleaner/modern `exp as $v | ...` syntax which is standard in both JQ 1.6+ and jaq.

### 3. Order of Definitions (Strictness)
*   **Status**: ⚠️ **STRICT**. `jaq` requires functions to be defined BEFORE they are used in subsequent definitions.
*   **Impact**: MEDIUM. Prevents circular dependencies.
*   **Evaluation**: **Stronger**. Forces cleaner modularization.

---

## 🔍 Pending Audit
| Feature | jaq Status | Notes |
| :--- | :--- | :--- |
| `input`/`inputs` | TBD | Affects `-n` vs streaming mode |
| `module`/`import` | PARTIAL | `jaq` supports modules but search paths differ |

---

## Rule of Thumb for Developers
> `jaq` is high-performance and safe, but strict. Use **ISO8601** timestamps and **`as`** bindings to stay compatible.
