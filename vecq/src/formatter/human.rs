use crate::error::VecqResult;
use serde_json::Value;
use super::{OutputFormatter, FormatOptions};
use std::fmt::Write;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_human_formatter() {
        let formatter = HumanFormatter::new();
        let data = json!([
            { "name": "main", "type": "function" },
            { "name": "helper", "type": "function" }
        ]);
        
        let mut options = FormatOptions::human_readable();
        options.color_output = false; 
        let result = formatter.format(&data, &options).unwrap();
        
        // Should contain table-like structure
        assert!(result.contains("name"));
        assert!(result.contains("main"));
        assert!(result.contains("helper"));
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
        assert!(result.contains("42"));
    }
}
