def cargo_to_artifact:
  # Input: Cargo JSON message
  select(.reason == "compiler-message")
  | {
      type: (if .message.level == "error" then "BUILD_ERROR" elif .message.level == "warning" then "COMPILATION_WARNING" else "INFO" end),
      source: "cargo",
      status: (if .message.level == "error" then "FAILURE" else "WARNING" end),
      summary: .message.message,
      details: .message.rendered,
      location: {
        file: .message.spans[0].file_name,
        line: .message.spans[0].line_start,
        column: .message.spans[0].column_start,
        end_line: .message.spans[0].line_end,
        end_column: .message.spans[0].column_end
      },
      check_name: .message.code.code
    };
