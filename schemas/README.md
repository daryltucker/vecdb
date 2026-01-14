# vecdb Schemas

This directory contains **canonical schemas** for common data types. Schemas define the contract between normalizers (transforms raw data → schema) and renderers (transforms schema → output).

## Philosophy

> *"Parse, Don't Validate"* — The schema IS the contract.

We provide **lenient normalizer examples**, not strict validators. Users fork and customize for their edge cases.

## Available Schemas

| Schema | Description | Normalizers |
|--------|-------------|-------------|
| `log.schema.json` | Chronological events (logs) | `nginx_to_log`, `journald_to_log` |
| `task.schema.json` | Project management tasks | `github_to_task`, `todo_to_task`, `src_to_task` |
| `artifact.schema.json` | Build/Test artifacts | `cargo_to_artifact`, `junit_to_artifact` |
| `diff.schema.json` | Code changes and hunks | `git_diff_to_diff` |
| `chat.schema.json` | Conversational messages | `openwebui_to_chat` |

## Extension Mechanism

All schemas support `x-` prefixed fields for custom metadata:

```json
{
  "role": "user",
  "content": "Hello",
  "x-source": "my-custom-app",
  "x-metadata": { "custom": "data" }
}
```

Extensions are **ignored by standard processors** and can be promoted to standard fields via RFC.

## Contributing

- **New Schema**: Open issue using the Schema Submission template
- **New Normalizer**: Open issue using the Normalizer Submission template

## Usage

```bash
# Transform raw data → canonical schema → formatted output
vecq -L examples raw.json -q 'webui_to_chat | chat_format'

# Use just the normalizer for further processing
vecq -L examples raw.json -q 'webui_to_chat | .[-5:]'
```

## See Also

*   [Querying & Filtering Guide](../docs/examples/QUERYING.md)
*   [Thunar Integration](../docs/examples/THUNAR_INTEGRATION.md)
*   `vecq man --agent` : Agent-focused documentation

## See Also

*   [Querying & Filtering Guide](../docs/examples/QUERYING.md)
*   [Thunar Integration](../docs/examples/THUNAR_INTEGRATION.md)
*   `vecq man --agent` : Agent-focused documentation
