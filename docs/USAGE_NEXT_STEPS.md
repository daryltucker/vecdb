# Usage Detection Feature: Current State & Next Steps

## Session Context
**Date**: January 23, 2026 (Friday)
**Status**: Usage detection partially implemented at parser level, missing jq integration
**Goal**: Complete end-to-end usage querying with `vecq --enable-usages file.rs '.usages[]'`

## Current Implementation Status

### ✅ What Exists (Parser Level)
- **Parser Support**: Rust, Go, Python, JavaScript parsers have `detect_usages()` methods
- **Element Types**: `FunctionCall`, `VariableReference`, `TypeReference`, `MethodCall`, `Assignment`, `ImportUsage`
- **Attributes**: `UsageAttributes` struct with `symbol_name`, `usage_type`, `context`, `scope` fields
- **CLI Flag**: `--enable-usages` flag exists in vecq CLI arguments
- **Tests**: Usage detection tests exist in parser modules

### ❌ What's Missing (Query Integration)
- **JQ Filters**: No `.usages[]` filter in stdlib for querying usage elements
- **Pipeline Integration**: Usage elements not included in JSON output for jq querying
- **Query Workflow**: `vecq --enable-usages file.rs '.usages[]'` doesn't work
- **Documentation**: `docs/internal/USAGE.md` doesn't exist
- **Examples**: No CLI help text showing usage query patterns

## Implementation Gap Analysis

```
Parser Level: ✅ Usage detection works, creates elements
Query Level: ❌ No .usages[] filter, no jq integration
CLI Level: ⚠️ --enable-usages flag exists but doesn't enable queries
User Level: ❌ No way to query usages via vecq commands
```

## Planned Implementation Phases

### Phase 1: JQ Filter Implementation
Add usage filters to `vecq/src/stdlib/auto.jq`:
```jq
# Filter usage elements by type
def usages: .elements[] | select(.element_type | contains("Call") or contains("Reference") or contains("Assignment") or contains("Import"));
def calls: usages | select(.element_type | contains("Call"));
def references: usages | select(.element_type | contains("Reference"));
def assignments: usages | select(.element_type | contains("Assignment"));
def imports: usages | select(.element_type | contains("Import"));
```

### Phase 2: Query Pipeline Integration
Modify `vecq/src/converter.rs` to include usage elements in JSON output:
```rust
if options.enable_usages {
    let usage_elements = parser.detect_usages(content, ast, None, "")?;
    document.elements.extend(usage_elements);
}
```

### Phase 3: CLI Integration
- Update help text with usage query examples
- Add validation for usage queries without --enable-usages flag
- Create usage query presets

### Phase 4: Testing & Documentation
- Add integration tests for end-to-end usage queries
- Create comprehensive USAGE.md documentation
- Update CLI examples and help text

## Tradeoff Decisions Needed

### 1. Auto-Detection vs Explicit Flag
**Question**: Should usage queries automatically enable usage detection?
- **Option A**: Auto-detect when `.usages[]` appears in query → automatically enable usage detection
- **Option B**: Require explicit `--enable-usages` flag → user must know to enable it

**Tradeoffs**:
- Auto-detection: Better UX, no need to remember flag
- Explicit flag: Performance control, user awareness of cost

### 2. Filter Granularity
**Question**: How specific should jq filters be?
- **Option A**: Single `.usages[]` filter with jq selection (`.usages[] | select(.usage_type == "call")`)
- **Option B**: Multiple specific filters (`.calls[]`, `.references[]`, `.assignments[]`, `.imports[]`)

**Tradeoffs**:
- Single filter: More flexible jq queries, follows existing patterns
- Multiple filters: More discoverable, simpler for common cases

### 3. Performance Considerations
**Question**: Acceptable overhead when usage detection enabled?
- **Current estimate**: ~2x parsing time
- **Impact**: Only affects files with --enable-usages flag

### 4. Language Scope
**Question**: Which parsers to prioritize for usage detection?
- **Current**: Rust, Go, Python, JavaScript have detection code
- **Priority order**: Rust first (most mature), then Python, then others

## Success Criteria

### Functional
- ✅ `vecq --enable-usages file.rs '.usages[]'` returns usage elements
- ✅ Usage queries work across supported languages
- ✅ Performance impact < 2x when enabled

### Usability
- ✅ Clear error messages when usage queries used without flag
- ✅ Helpful CLI help text with examples
- ✅ Intuitive filter naming and behavior

### Quality
- ✅ Comprehensive test coverage for usage queries
- ✅ Documentation with examples and best practices
- ✅ Clean integration with existing vecq workflows

## Next Session Action Items

1. **Decide on auto-detection**: Auto-detect usage queries or require explicit flag?
2. **Choose filter design**: Single .usages[] filter vs multiple specific filters?
3. **Implement jq filters**: Add chosen filters to stdlib
4. **Integrate pipeline**: Modify JSON output to include usage elements
5. **Test integration**: Verify end-to-end functionality
6. **Create documentation**: Write comprehensive USAGE.md

## Implementation Notes

### Current Code Structure
- Usage detection: `vecq/src/parsers/*/usage.rs` (where exists)
- Element types: `vecq/src/types/element_type.rs`
- Attributes: `vecq/src/types/attributes.rs`
- CLI args: `vecq/src/cli/args.rs`
- JQ stdlib: `vecq/src/stdlib/auto.jq`

### Key Integration Points
- `vecq/src/converter.rs`: Where usage elements need to be added to JSON
- `vecq/src/cli/output.rs`: Where query results are processed
- `vecq/src/query.rs`: Where jq filters are defined

### Testing Strategy
- Unit tests: Parser-level usage detection (already exists)
- Integration tests: End-to-end usage queries (needs to be added)
- Performance tests: Benchmark parsing with/without usage detection

---

*This document captures the current state and decisions needed to complete the usage detection feature. Created January 23, 2026.*</content>
<parameter name="filePath">/home/daryl/Projects/NRG/vecdb-mcp/docs/USAGE_NEXT_STEPS.md