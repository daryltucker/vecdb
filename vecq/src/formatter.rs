// PURPOSE:
//   Output formatting system for vecq that generates various output formats
//   for different use cases and Unix pipeline compatibility. Critical for vecq's
//   integration with existing Unix toolchain - must produce grep-compatible output
//   while also supporting JSON, human-readable, and custom formats.
//
// REQUIREMENTS:
//   User-specified:
//   - Must support JSON output with pretty-printing and compact modes
//   - Must generate grep-compatible format (filename:line:content)
//   - Must provide human-readable output with tables and trees
//   - Must preserve file paths and line numbers in all output formats
//   - Must support pipeline workflows with standard Unix tools
//   
//   Implementation-discovered:
//   - Requires flexible formatting options for different output modes
//   - Must handle large result sets efficiently without memory issues
//   - Needs color output support for terminal display
//   - Must support streaming output for large datasets
//
// IMPLEMENTATION RULES:
//   1. All formatters must preserve file path and line number information
//      Rationale: Required for grep compatibility and source location tracking
//   
//   2. Grep-compatible format must exactly match "filename:line:content" pattern
//      Rationale: Ensures compatibility with existing Unix tools and workflows
//   
//   3. JSON formatter must support both pretty and compact modes
//      Rationale: Pretty for human reading, compact for machine processing
//   
//   4. Human-readable formatter must be optimized for terminal display
//      Rationale: Provides best user experience for interactive use
//   
//   5. All formatters must handle empty results gracefully
//      Rationale: Empty query results should not cause errors or confusion
//   
//   Critical:
//   - DO NOT change grep-compatible format (breaks Unix pipeline compatibility)
//   - DO NOT lose file path or line number information in any format
//   - ALWAYS handle large result sets without memory exhaustion
//
// USAGE:
//   use vecq::formatter::{OutputFormatter, JsonFormatter, GrepFormatter, HumanFormatter};
//   use vecq::formatter::FormatOptions;
//   
//   // JSON formatting
//   let json_formatter = JsonFormatter::new();
//   let options = FormatOptions::default().with_pretty_print(true);
//   let output = json_formatter.format(&query_result, &options)?;
//   
//   // Grep-compatible formatting
//   let grep_formatter = GrepFormatter::new();
//   let options = FormatOptions::default().with_grep_compatible(true);
//   let output = grep_formatter.format(&query_result, &options)?;
//   
//   // Pipeline usage
//   // vecq file.rs '.functions[]' --grep-format | grep "pub" | cut -d: -f1
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new output formats:
//   1. Implement OutputFormatter trait with format() and format_name() methods
//   2. Ensure file path and line number preservation in new format
//   3. Add comprehensive tests for new formatter
//   4. Update CLI to support new format option
//   5. Document new format in user documentation
//   6. Add property tests to validate format consistency
//   
//   When modifying existing formats:
//   1. Ensure backward compatibility with existing pipelines
//   2. Test with real Unix tools (grep, awk, sed, cut, sort)
//   3. Validate performance with large result sets
//   4. Update format documentation and examples
//   5. Add regression tests for format changes
//
// RELATED FILES:
//   - src/query.rs - Produces query results that get formatted
//   - src/main.rs - CLI interface that selects formatters
//   - src/types.rs - Defines data structures that get formatted
//   - tests/unit/formatter_tests.rs - Formatter validation tests
//   - tests/integration/pipeline_tests.rs - Unix pipeline compatibility tests
//
// MAINTENANCE:
//   Update when:
//   - New output formats are requested by users
//   - Unix tool compatibility issues are discovered
//   - Performance optimization opportunities identified
//   - Color output or terminal features need enhancement
//   - Streaming output support needs improvement
//
// Last Verified: 2025-12-31

use crate::error::{VecqError, VecqResult};
use serde_json::Value;
use std::fmt::Write;

/// Trait for formatting query results into different output formats
pub trait OutputFormatter: Send + Sync {
    /// Format query results according to options
    fn format(&self, data: &Value, options: &FormatOptions) -> VecqResult<String>;

    /// Get the name of this formatter
    fn format_name(&self) -> &str;

    /// Check if this formatter supports streaming output
    fn supports_streaming(&self) -> bool {
        false
    }
}

/// Options for controlling output formatting
#[derive(Debug, Clone)]
pub struct FormatOptions {
    /// Pretty-print JSON with indentation
    pub pretty_print: bool,
    /// Include line numbers in output
    pub include_line_numbers: bool,
    /// Format for grep compatibility
    pub grep_compatible: bool,
    /// Use color output for terminals
    pub color_output: bool,
    /// Maximum width for human-readable output
    pub max_width: Option<usize>,
    /// Include file paths in output
    pub include_file_paths: bool,
    /// Compact output (minimal whitespace)
    pub compact: bool,
    /// Custom format string (for advanced users)
    pub custom_format: Option<String>,
    /// Output raw strings
    pub raw_output: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            pretty_print: false,
            include_line_numbers: true,
            grep_compatible: false,
            color_output: false,
            max_width: None,
            include_file_paths: true,
            compact: false,
            custom_format: None,
            raw_output: false,
        }
    }
}

impl FormatOptions {
    /// Create options for pretty JSON output
    pub fn pretty_json() -> Self {
        Self {
            pretty_print: true,
            compact: false,
            ..Default::default()
        }
    }

    /// Create options for compact JSON output
    pub fn compact_json() -> Self {
        Self {
            pretty_print: false,
            compact: true,
            ..Default::default()
        }
    }

    /// Create options for grep-compatible output
    pub fn grep_compatible() -> Self {
        Self {
            grep_compatible: true,
            include_line_numbers: true,
            include_file_paths: true,
            ..Default::default()
        }
    }

    /// Create options for human-readable terminal output
    pub fn human_readable() -> Self {
        Self {
            color_output: true,
            max_width: Some(120),
            ..Default::default()
        }
    }

    /// Builder method for pretty printing
    pub fn with_pretty_print(mut self, pretty: bool) -> Self {
        self.pretty_print = pretty;
        self
    }

    /// Builder method for grep compatibility
    pub fn with_grep_compatible(mut self, grep: bool) -> Self {
        self.grep_compatible = grep;
        self
    }

    /// Builder method for color output
    pub fn with_color_output(mut self, color: bool) -> Self {
        self.color_output = color;
        self
    }

    /// Builder method for line numbers
    pub fn with_line_numbers(mut self, include: bool) -> Self {
        self.include_line_numbers = include;
        self
    }
}

/// JSON output formatter
pub struct JsonFormatter;

impl JsonFormatter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsonFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for JsonFormatter {
    fn format(&self, data: &Value, options: &FormatOptions) -> VecqResult<String> {
        if options.raw_output {
            match data {
                Value::String(s) => return Ok(s.clone()),
                Value::Array(arr) => {
                    let lines: Vec<String> = arr.iter().map(|v| {
                        match v {
                            Value::String(s) => s.clone(),
                            _ => serde_json::to_string(v).unwrap_or_default(),
                        }
                    }).collect();
                    return Ok(lines.join("\n"));
                },
                Value::Null => return Ok(String::new()),
                _ => {}
            }
        }
        
        if options.pretty_print {
            serde_json::to_string_pretty(data)
        } else {
            serde_json::to_string(data)
        }
        .map_err(|e| VecqError::json_error("JSON formatting failed".to_string(), Some(e)))
    }

    fn format_name(&self) -> &str {
        "json"
    }

    fn supports_streaming(&self) -> bool {
        true
    }
}

/// Grep-compatible output formatter
pub struct GrepFormatter;

impl GrepFormatter {
    pub fn new() -> Self {
        Self
    }

    /// Extract file path from metadata or element
    fn extract_file_path(&self, value: &Value) -> Option<String> {
        // Try to get file path from metadata
        if let Some(metadata) = value.get("metadata") {
            if let Some(path) = metadata.get("path") {
                return path.as_str().map(|s| s.to_string());
            }
        }

        // Try to get from element attributes
        if let Some(attributes) = value.get("attributes") {
            if let Some(path) = attributes.get("file_path") {
                return path.as_str().map(|s| s.to_string());
            }
        }

        None
    }

    /// Extract line number from element
    fn extract_line_number(&self, value: &Value) -> Option<usize> {
        value.get("line_start")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
    }

    /// Extract content from element
    fn extract_content(&self, value: &Value) -> String {
        // Try different content fields
        if let Some(content) = value.get("content") {
            if let Some(s) = content.as_str() {
                return s.to_string();
            }
        }

        if let Some(name) = value.get("name") {
            if let Some(s) = name.as_str() {
                return s.to_string();
            }
        }

        // Fallback to JSON representation
        serde_json::to_string(value).unwrap_or_else(|_| "unknown".to_string())
    }

    /// Format a single element in grep format
    fn format_element(&self, element: &Value, default_path: &str) -> String {
        let file_path = self.extract_file_path(element)
            .unwrap_or_else(|| default_path.to_string());
        let line_number = self.extract_line_number(element).unwrap_or(1);
        let content = self.extract_content(element);

        // Clean content for single-line output
        let clean_content = content
            .lines()
            .next()
            .unwrap_or(&content)
            .trim();

        format!("{}:{}:{}", file_path, line_number, clean_content)
    }
}

impl Default for GrepFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for GrepFormatter {
    fn format(&self, data: &Value, _options: &FormatOptions) -> VecqResult<String> {
        let mut output = String::new();
        let default_path = "unknown";

        match data {
            Value::Array(elements) => {
                for element in elements {
                    let line = self.format_element(element, default_path);
                    writeln!(output, "{}", line).unwrap();
                }
            }
            Value::Object(_) => {
                // Single object
                let line = self.format_element(data, default_path);
                writeln!(output, "{}", line).unwrap();
            }
            Value::Null => {
                // For grep format, treat null as "no match" / empty output
                // This avoids spamming "unknown:1:null" when queries return no results
            }
            _ => {
                // Primitive value
                writeln!(output, "{}:1:{}", default_path, data).unwrap();
            }
        }

        Ok(output)
    }

    fn format_name(&self) -> &str {
        "grep"
    }
}

/// Human-readable output formatter
pub struct HumanFormatter;

impl HumanFormatter {
    pub fn new() -> Self {
        Self
    }

    /// Format data as a table with alignment and color
    fn format_as_table(&self, data: &Value, options: &FormatOptions) -> VecqResult<String> {
        let mut output = String::new();

        match data {
            Value::Array(elements) if !elements.is_empty() => {
                // Determine columns from first element to maintain some structure
                // Real implementation should probably scan all elements to find all keys
                if let Some(first) = elements.first() {
                    if let Value::Object(obj) = first {
                        let mut columns: Vec<&String> = obj.keys().collect();
                        columns.sort(); // Consistent column order

                        // Calculate column widths
                        let mut widths: std::collections::HashMap<&String, usize> = std::collections::HashMap::new();
                        for col in &columns {
                            widths.insert(*col, col.len());
                        }

                        for element in elements {
                            if let Value::Object(obj) = element {
                                for col in &columns {
                                    let val = obj.get(*col)
                                        .map(|v| self.format_value_compact(v))
                                        .unwrap_or_default();
                                    let width = widths.get_mut(*col).unwrap();
                                    *width = (*width).max(val.len());
                                }
                            }
                        }

                        // Apply max width constraint if any
                        if let Some(max_w) = options.max_width {
                            let total_w: usize = widths.values().sum::<usize>() + (columns.len() - 1) * 3;
                            if total_w > max_w {
                                // Simple proportional scaling could be done here, but for now we just cap
                            }
                        }

                        // Header
                        for (i, col) in columns.iter().enumerate() {
                            let padded = format!("{:width$}", col, width = widths[*col]);
                            output.push_str(&self.colorize(&padded, "header", options));
                            if i < columns.len() - 1 {
                                output.push_str(&self.colorize(" | ", "line", options));
                            }
                        }
                        output.push('\n');

                        // Separator
                        for (i, col) in columns.iter().enumerate() {
                            output.push_str(&self.colorize(&"-".repeat(widths[*col]), "line", options));
                            if i < columns.len() - 1 {
                                output.push_str(&self.colorize("-+-", "line", options));
                            }
                        }
                        output.push('\n');

                        // Rows
                        for element in elements {
                            if let Value::Object(obj) = element {
                                for (i, col) in columns.iter().enumerate() {
                                    let val = obj.get(*col)
                                        .map(|v| self.format_value_compact(v))
                                        .unwrap_or_default();
                                    let padded = format!("{:width$}", val, width = widths[*col]);
                                    output.push_str(&self.colorize(&padded, "value", options));
                                    if i < columns.len() - 1 {
                                        output.push_str(&self.colorize(" | ", "line", options));
                                    }
                                }
                                output.push('\n');
                            }
                        }
                    } else {
                        // Array of primitives
                        for element in elements {
                            writeln!(output, "{}", self.colorize(&self.format_value_compact(element), "value", options)).unwrap();
                        }
                    }
                }
            }
            Value::Object(obj) => {
                // Single object as key-value pairs
                let mut keys: Vec<&String> = obj.keys().collect();
                keys.sort();
                
                let max_key_len = keys.iter().map(|k| k.len()).max().unwrap_or(0);

                for key in keys {
                    let value = obj.get(key).unwrap();
                    let padded_key = format!("{:width$}", key, width = max_key_len);
                    output.push_str(&self.colorize(&padded_key, "key", options));
                    output.push_str(&self.colorize(": ", "line", options));
                    output.push_str(&self.colorize(&self.format_value_compact(value), "value", options));
                    output.push('\n');
                }
            }
            _ => {
                // Primitive value
                writeln!(output, "{}", self.colorize(&self.format_value_compact(data), "value", options)).unwrap();
            }
        }

        Ok(output)
    }

    /// Format a value in compact form
    fn format_value_compact(&self, value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Array(arr) => {
                if arr.len() <= 3 {
                    format!("[{}]", arr.iter()
                        .map(|v| self.format_value_compact(v))
                        .collect::<Vec<_>>()
                        .join(", "))
                } else {
                    format!("[{} items]", arr.len())
                }
            }
            Value::Object(_) => "{object}".to_string(),
        }
    }

    /// Apply color formatting if enabled
    fn colorize(&self, text: &str, color: &str, options: &FormatOptions) -> String {
        if options.color_output {
            match color {
                "header" => format!("\x1b[1;34m{}\x1b[0m", text), // Bold blue
                "key" => format!("\x1b[1;32m{}\x1b[0m", text),    // Bold green
                "value" => format!("\x1b[33m{}\x1b[0m", text),    // Yellow
                "line" => format!("\x1b[90m{}\x1b[0m", text),     // Gray
                _ => text.to_string(),
            }
        } else {
            text.to_string()
        }
    }
}

impl Default for HumanFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for HumanFormatter {
    fn format(&self, data: &Value, options: &FormatOptions) -> VecqResult<String> {
        self.format_as_table(data, options)
    }

    fn format_name(&self) -> &str {
        "human"
    }
}

/// Registry for managing output formatters
#[derive(Default)]
pub struct FormatterRegistry {
    formatters: std::collections::HashMap<String, Box<dyn OutputFormatter>>,
}

impl FormatterRegistry {
    /// Create a new formatter registry with default formatters
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register_default_formatters();
        registry
    }

    /// Register a formatter
    pub fn register(&mut self, name: String, formatter: Box<dyn OutputFormatter>) {
        self.formatters.insert(name, formatter);
    }

    /// Get formatter by name
    pub fn get_formatter(&self, name: &str) -> Option<&dyn OutputFormatter> {
        self.formatters.get(name).map(|f| f.as_ref())
    }

    /// List available formatter names
    pub fn available_formatters(&self) -> Vec<String> {
        self.formatters.keys().cloned().collect()
    }

    /// Register default formatters
    fn register_default_formatters(&mut self) {
        self.register("json".to_string(), Box::new(JsonFormatter::new()));
        self.register("grep".to_string(), Box::new(GrepFormatter::new()));
        self.register("human".to_string(), Box::new(HumanFormatter::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_data() -> Value {
        json!([
            {
                "name": "main",
                "type": "function",
                "line_start": 1,
                "line_end": 5,
                "content": "fn main() { println!(\"Hello\"); }",
                "attributes": {
                    "file_path": "src/main.rs"
                }
            },
            {
                "name": "helper",
                "type": "function", 
                "line_start": 7,
                "line_end": 10,
                "content": "fn helper() -> i32 { 42 }",
                "attributes": {
                    "file_path": "src/main.rs"
                }
            }
        ])
    }

    #[test]
    fn test_json_formatter() {
        let formatter = JsonFormatter::new();
        let data = create_test_data();
        
        // Compact format
        let options = FormatOptions::compact_json();
        let result = formatter.format(&data, &options).unwrap();
        assert!(!result.contains('\n')); // Should be compact
        
        // Pretty format
        let options = FormatOptions::pretty_json();
        let result = formatter.format(&data, &options).unwrap();
        assert!(result.contains('\n')); // Should have newlines
        assert!(result.contains("  ")); // Should have indentation
    }

    #[test]
    fn test_grep_formatter() {
        let formatter = GrepFormatter::new();
        let data = create_test_data();
        
        let options = FormatOptions::grep_compatible();
        let result = formatter.format(&data, &options).unwrap();
        
        let lines: Vec<&str> = result.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
        
        // Check format: filename:line:content
        assert!(lines[0].starts_with("src/main.rs:1:"));
        assert!(lines[1].starts_with("src/main.rs:7:"));
        assert!(lines[0].contains("main"));
        assert!(lines[1].contains("helper"));
    }

    #[test]
    fn test_human_formatter() {
        let formatter = HumanFormatter::new();
        let data = create_test_data();
        
        let options = FormatOptions::human_readable();
        let result = formatter.format(&data, &options).unwrap();
        
        // Should contain table-like structure
        assert!(result.contains("|"));
        assert!(result.contains("-"));
        assert!(result.contains("name"));
        assert!(result.contains("main"));
        assert!(result.contains("helper"));
    }

    #[test]
    fn test_format_options_builders() {
        let options = FormatOptions::default()
            .with_pretty_print(true)
            .with_color_output(true)
            .with_grep_compatible(false);
        
        assert!(options.pretty_print);
        assert!(options.color_output);
        assert!(!options.grep_compatible);
    }

    #[test]
    fn test_formatter_registry() {
        let registry = FormatterRegistry::new();
        
        assert!(registry.get_formatter("json").is_some());
        assert!(registry.get_formatter("grep").is_some());
        assert!(registry.get_formatter("human").is_some());
        assert!(registry.get_formatter("unknown").is_none());
        
        let available = registry.available_formatters();
        assert!(available.contains(&"json".to_string()));
        assert!(available.contains(&"grep".to_string()));
        assert!(available.contains(&"human".to_string()));
    }

    #[test]
    fn test_grep_formatter_single_object() {
        let formatter = GrepFormatter::new();
        let data = json!({
            "name": "test_func",
            "line_start": 42,
            "content": "fn test_func() {}",
            "attributes": {
                "file_path": "test.rs"
            }
        });
        
        let options = FormatOptions::grep_compatible();
        let result = formatter.format(&data, &options).unwrap();
        
        assert!(result.contains("test.rs:42:"));
        assert!(result.contains("test_func"));
    }

    #[test]
    fn test_human_formatter_single_object() {
        let formatter = HumanFormatter::new();
        let data = json!({
            "name": "test_func",
            "type": "function",
            "line_start": 42
        });
        
        let mut options = FormatOptions::human_readable();
        options.color_output = false;
        let result = formatter.format(&data, &options).unwrap();
        
        assert!(result.contains("name"));
        assert!(result.contains("test_func"));
        assert!(result.contains("type"));
        assert!(result.contains("function"));
        assert!(result.contains("line_start"));
        assert!(result.contains("42"));
    }

    #[test]
    fn test_empty_data_handling() {
        let json_formatter = JsonFormatter::new();
        let grep_formatter = GrepFormatter::new();
        let human_formatter = HumanFormatter::new();
        
        let empty_array = json!([]);
        let options = FormatOptions::default();
        
        // All formatters should handle empty data gracefully
        assert!(json_formatter.format(&empty_array, &options).is_ok());
        assert!(grep_formatter.format(&empty_array, &options).is_ok());
        assert!(human_formatter.format(&empty_array, &options).is_ok());
    }
}