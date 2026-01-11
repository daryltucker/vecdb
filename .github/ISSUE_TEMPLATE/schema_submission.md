---
name: Schema Submission
about: Propose a new canonical schema
title: '[SCHEMA] '
labels: schema, enhancement
assignees: ''
---

## Schema Name
<!-- e.g., "log", "event", "commit" -->

## Description
<!-- What data type does this schema represent? -->

## Use Cases
<!-- Who would use this schema and for what? -->

## Proposed Structure
```json
{
  "required_field": "...",
  "optional_field": "..."
}
```

## Existing Formats
<!-- What raw formats would normalizers convert FROM? -->
- Format 1: ...
- Format 2: ...

## Prior Art
<!-- Are there existing standards we should consider? (JSON Schema, Schema.org, etc.) -->

## Checklist
- [ ] I have searched existing schemas to avoid duplication
- [ ] I am willing to contribute a reference normalizer
- [ ] I have considered backward compatibility with potential future versions
