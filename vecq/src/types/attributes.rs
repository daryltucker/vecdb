use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Attributes specific to Rust elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RustAttributes {
    pub visibility: String,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to TOML elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TomlAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to JavaScript elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JavaScriptAttributes {
    pub is_async: bool,
    pub is_arrow: bool,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to JSON elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Python elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PythonAttributes {
    pub is_async: bool,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Go elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to C/C++/CUDA elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CFamilyAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Bash elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BashAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to Markdown elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarkdownAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to HTML elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HtmlAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Attributes specific to plain text elements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextAttributes {
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

/// Container for element-specific attributes
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)] // Serializes content directly, no wrapping key
pub enum ElementAttributes {
    Rust(RustAttributes),
    Toml(TomlAttributes),
    JavaScript(JavaScriptAttributes),
    Json(JsonAttributes),
    Python(PythonAttributes),
    Go(GoAttributes),
    CFamily(CFamilyAttributes),
    Bash(BashAttributes),
    Markdown(MarkdownAttributes),
    Html(HtmlAttributes),
    Text(TextAttributes),
    Generic(HashMap<String, serde_json::Value>),
}

impl Default for ElementAttributes {
    fn default() -> Self {
        ElementAttributes::Generic(HashMap::new())
    }
}

impl ElementAttributes {
    /// Helper to insert into the generic map or flattened other map
    pub fn insert_generic(&mut self, key: String, value: serde_json::Value) {
        match self {
            Self::Generic(map) => { map.insert(key, value); },
            Self::Rust(attr) => { attr.other.insert(key, value); },
            Self::Toml(attr) => { attr.other.insert(key, value); },
            Self::JavaScript(attr) => { attr.other.insert(key, value); },
            Self::Json(attr) => { attr.other.insert(key, value); },
            Self::Python(attr) => { attr.other.insert(key, value); },
            Self::Go(attr) => { attr.other.insert(key, value); },
            Self::CFamily(attr) => { attr.other.insert(key, value); },
            Self::Bash(attr) => { attr.other.insert(key, value); },
            Self::Markdown(attr) => { attr.other.insert(key, value); },
            Self::Html(attr) => { attr.other.insert(key, value); },
            Self::Text(attr) => { attr.other.insert(key, value); },
        }
    }
    
    /// Helper to get generic value
    pub fn get_text(&self, key: &str) -> Option<String> {
        match self {
            Self::Generic(map) => map.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Rust(attr) => {
                if key == "visibility" { return Some(attr.visibility.clone()); }
                attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
            },
            Self::Toml(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::JavaScript(attr) => {
                if key == "is_async" { return Some(attr.is_async.to_string()); }
                if key == "is_arrow" { return Some(attr.is_arrow.to_string()); }
                attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
            },
            Self::Json(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Python(attr) => {
                if key == "is_async" { return Some(attr.is_async.to_string()); }
                attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
            },
            Self::Go(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::CFamily(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Bash(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Markdown(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Html(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
            Self::Text(attr) => attr.other.get(key).and_then(|v| v.as_str()).map(|s| s.to_string()),
        }
    }

    /// Check if attributes are empty
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Generic(map) => map.is_empty(),
            Self::Rust(attr) => attr.visibility == "private" && attr.other.is_empty(), // Assume private is default?
            Self::Toml(attr) => attr.other.is_empty(),
            Self::JavaScript(attr) => !attr.is_async && !attr.is_arrow && attr.other.is_empty(),
            Self::Json(attr) => attr.other.is_empty(),
            Self::Python(attr) => !attr.is_async && attr.other.is_empty(),
            Self::Go(attr) => attr.other.is_empty(),
            Self::CFamily(attr) => attr.other.is_empty(),
            Self::Bash(attr) => attr.other.is_empty(),
            Self::Markdown(attr) => attr.other.is_empty(),
            Self::Html(attr) => attr.other.is_empty(),
            Self::Text(attr) => attr.other.is_empty(),
        }
    }

    /// Get generic value helper
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        match self {
            Self::Generic(map) => map.get(key),
            Self::Rust(attr) => attr.other.get(key),
            Self::Toml(attr) => attr.other.get(key),
            Self::JavaScript(attr) => attr.other.get(key),
            Self::Json(attr) => attr.other.get(key),
            Self::Python(attr) => attr.other.get(key),
            Self::Go(attr) => attr.other.get(key),
            Self::CFamily(attr) => attr.other.get(key),
            Self::Bash(attr) => attr.other.get(key),
            Self::Markdown(attr) => attr.other.get(key),
            Self::Html(attr) => attr.other.get(key),
            Self::Text(attr) => attr.other.get(key),
        }
    }

    /// Check if key exists
    pub fn contains_key(&self, key: &str) -> bool {
        match self {
            Self::Generic(map) => map.contains_key(key),
            Self::Rust(attr) => key == "visibility" || attr.other.contains_key(key),
            Self::Toml(attr) => attr.other.contains_key(key),
            Self::JavaScript(attr) => key == "is_async" || key == "is_arrow" || attr.other.contains_key(key),
            Self::Json(attr) => attr.other.contains_key(key),
            Self::Python(attr) => key == "is_async" || attr.other.contains_key(key),
            Self::Go(attr) => attr.other.contains_key(key),
            Self::CFamily(attr) => attr.other.contains_key(key),
            Self::Bash(attr) => attr.other.contains_key(key),
            Self::Markdown(attr) => attr.other.contains_key(key),
            Self::Html(attr) => attr.other.contains_key(key),
            Self::Text(attr) => attr.other.contains_key(key),
        }
    }
}
