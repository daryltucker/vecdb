use crate::error::{VecqError, VecqResult};
use serde_json::Value;
use super::{OutputFormatter, FormatOptions};

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_json_formatter() {
        let formatter = JsonFormatter::new();
        let data = json!([
            { "name": "test" }
        ]);
        
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
    fn test_empty_data_handling() {
        let formatter = JsonFormatter::new();
        let empty_array = json!([]);
        let options = FormatOptions::default();
        assert!(formatter.format(&empty_array, &options).is_ok());
    }
}
