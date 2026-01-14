use crate::error::VecqResult;
use serde_json::Value;
use super::{OutputFormatter, FormatOptions};
use std::fmt::Write;

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_grep_formatter() {
        let formatter = GrepFormatter::new();
        let data = json!([
            {
                "name": "main",
                "content": "fn main() {}",
                "line_start": 1,
                "attributes": { "file_path": "src/main.rs" }
            }
        ]);
        
        let options = FormatOptions::grep_compatible();
        let result = formatter.format(&data, &options).unwrap();
        
        assert!(result.contains("src/main.rs:1:"));
        assert!(result.contains("main"));
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
}
