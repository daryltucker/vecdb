use crate::error::VecqResult;
use serde_json::Value;

pub mod json;
pub mod grep;
pub mod human;

pub use json::JsonFormatter;
pub use grep::GrepFormatter;
pub use human::HumanFormatter;

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
}
