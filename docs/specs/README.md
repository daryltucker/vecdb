# Specifications Strategy

The `docs/specs/` directory contains detailed technical specifications for specific sub-systems or features of Vecdb-MCP.

**Purpose**:
To separate high-level user documentation (in `docs/`) from low-level implementation details and rigorous specs.

**Contents**:
*   `CHUNKING_STRATEGY.md`: detailed algorithms for text chunking.
*   `PARSING_GUIDELINES.md`: rules for the `vecq` parsers.

**When to add a Spec**:
*   When a feature has complex internal logic (e.g., a specific ranking algorithm).
*   When defining a strict interface or format (e.g., a custom file format).
*   When behavior must be standardized across multiple implementations.
