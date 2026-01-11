use std::path::Path;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported file types for parsing and conversion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileType {
    Markdown,
    Rust,
    Python,
    C,
    Cpp,
    Cuda,
    Go,
    Bash,
    Json,
    Html,
    Toml,
    Text,

    Unknown,
}

impl FileType {
    /// Get file type from file extension
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_lowercase().as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "rs" => Some(Self::Rust),
            "py" | "pyw" => Some(Self::Python),
            "c" | "h" => Some(Self::C),
            "cpp" | "cc" | "cxx" | "hpp" | "hxx" => Some(Self::Cpp),
            "cu" | "cuh" => Some(Self::Cuda),
            "go" => Some(Self::Go),
            "sh" | "bash" => Some(Self::Bash),
            "json" | "ndjson" | "jsonl" => Some(Self::Json),
            "html" | "htm" | "xml" | "xhtml" => Some(Self::Html),
            "toml" => Some(Self::Toml),
            "txt" | "log" | "cfg" | "ini" | "conf" | "yaml" | "yml" => Some(Self::Text),

            _ => None,
        }
    }

    /// Get file type from file path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Self {
        let path = path.as_ref();
        
        // First try standard extension
        if let Some(ft) = path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Self::from_extension) 
        {
            return ft;
        }

        // Handle .resolved.N files (e.g. task.md.resolved.0 -> task.md)
        let path_str = path.to_string_lossy();
        if path_str.contains(".resolved.") {
            let parts: Vec<&str> = path_str.split('.').collect();
            // iterate backwards
            for i in (0..parts.len()).rev() {
                 if let Some(ft) = Self::from_extension(parts[i]) {
                     return ft;
                 }
            }
        }

        Self::Unknown
    }

    /// Get list of common extensions for this file type
    pub fn file_extensions(&self) -> Vec<&'static str> {
        match self {
            Self::Markdown => vec!["md", "markdown"],
            Self::Rust => vec!["rs"],
            Self::Python => vec!["py", "pyw"],
            Self::C => vec!["c", "h"],
            Self::Cpp => vec!["cpp", "cc", "cxx", "hpp", "hxx"],
            Self::Cuda => vec!["cu", "cuh"],
            Self::Go => vec!["go"],
            Self::Bash => vec!["sh", "bash"],
            Self::Json => vec!["json", "ndjson", "jsonl"],
            Self::Html => vec!["html", "htm", "xml", "xhtml"],
            Self::Toml => vec!["toml"],
            Self::Text => vec!["txt", "log", "cfg", "ini", "conf", "yaml", "yml"],
            Self::Unknown => vec![],
        }
    }

    /// Check if this file type is supported for parsing
    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// Check if content is likely text (not binary soup)
    /// Scans first 1KB for control characters or low printable ratio.
    pub fn is_likely_text(content: &[u8]) -> bool {
        if content.is_empty() { return true; }
        
        let sample_len = content.len().min(1024);
        let sample = &content[..sample_len];
        
        // Fast scan for null bytes (indicates binary)
        if sample.iter().any(|&b| b == 0) {
            return false;
        }
        
        // Check ratio of printable characters (including whitespace)
        let printable = sample.iter().filter(|&&b| {
            (b >= 32 && b <= 126) || b == 9 || b == 10 || b == 13
        }).count();
        
        (printable as f32 / sample_len as f32) > 0.85
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Markdown => "Markdown",
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::C => "C",
            Self::Cpp => "C++",
            Self::Cuda => "CUDA",
            Self::Go => "Go",
            Self::Bash => "Bash",
            Self::Json => "JSON",
            Self::Html => "HTML",
            Self::Toml => "TOML",
            Self::Text => "Text",

            Self::Unknown => "Unknown",
        };
        write!(f, "{}", name)
    }
}

/// Trait for detecting file types
/// This allows dependency injection of the detection logic into vecdb-core
pub trait FileTypeDetector: Send + Sync {
    /// Detect file type from path and content
    fn detect(&self, path: &Path, content: &[u8]) -> FileType;
}
