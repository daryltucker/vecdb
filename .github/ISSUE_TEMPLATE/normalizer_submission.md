---
name: Normalizer Submission
about: Contribute a normalizer for an existing schema
title: '[NORMALIZER] '
labels: normalizer, enhancement
assignees: ''
---

## Target Schema
<!-- Which canonical schema does this normalize TO? -->
- [ ] chat
- [ ] Other: ___

## Source Format
<!-- What raw format does this normalize FROM? -->
Name: 
Example source: <!-- Link to documentation or sample data -->

## Sample Input
```json
{
  "raw": "format",
  "example": "here"
}
```

## Sample Output
```json
{
  "role": "user",
  "content": "Normalized output",
  "timestamp": "2026-01-08T00:00:00Z"
}
```

## Implementation
<!-- Attach or paste your .jq file -->
```jq
def my_normalizer:
  # ...
```

## Behavior Notes
<!-- How does this handle edge cases? -->
- Empty fields: ...
- Missing timestamps: ...
- Malformed input: ...

## Checklist
- [ ] I have tested this against real data
- [ ] Output conforms to the target schema
- [ ] Extension fields use `x-` prefix
- [ ] I am willing to maintain this normalizer
