use serde::{Deserialize, Serialize};
use std::fmt;

/// Types of structural elements found in documents
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ElementType {
    // Universal elements
    Function,
    Class,
    Struct,
    Enum,
    Union,
    Interface,
    Module,
    Import,
    Variable,
    Constant,
    Comment,

    // Markdown-specific
    Header,
    CodeBlock,
    Link,
    Table,
    List,
    ListItem,
    Blockquote,
    Paragraph,
    HorizontalRule,
    Image,
    Emphasis,
    Strong,
    Strikethrough,
    FootnoteDefinition,
    HtmlElement,

    // Language-specific
    Trait,          // Rust
    Implementation, // Rust impl blocks
    Decorator,      // Python
    Macro,          // Rust, C/C++
    Namespace,      // C++
    Package,        // Go
    Kernel,         // CUDA __global__
    DeviceFunction, // CUDA __device__
    TypeAlias,      // Rust type alias

    // Usage/Reference types (new feature)
    FunctionCall,      // Function/method calls
    VariableReference, // Variable/constant references
    TypeReference,     // Type annotations and references
    MethodCall,        // Method calls on objects
    Assignment,        // Variable assignments
    ImportUsage,       // Import/re-export usages

    // Generic container
    Block,
    Unknown,
}

impl fmt::Display for ElementType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Function => "function",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Union => "union",
            Self::Interface => "interface",
            Self::Module => "module",
            Self::Import => "import",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Comment => "comment",
            Self::Header => "header",
            Self::CodeBlock => "code_block",
            Self::Link => "link",
            Self::Table => "table",
            Self::List => "list",
            Self::ListItem => "list_item",
            Self::Blockquote => "blockquote",
            Self::Paragraph => "paragraph",
            Self::HorizontalRule => "horizontal_rule",
            Self::Image => "image",
            Self::Emphasis => "emphasis",
            Self::Strong => "strong",
            Self::Strikethrough => "strikethrough",
            Self::FootnoteDefinition => "footnote_definition",
            Self::HtmlElement => "element",
            Self::Trait => "trait",
            Self::Implementation => "implementation",
            Self::Decorator => "decorator",
            Self::Macro => "macro",
            Self::Namespace => "namespace",
            Self::Package => "package",
            Self::Kernel => "kernel",
            Self::DeviceFunction => "device_function",
            Self::TypeAlias => "type_alias",
            Self::FunctionCall => "function_call",
            Self::VariableReference => "variable_reference",
            Self::TypeReference => "type_reference",
            Self::MethodCall => "method_call",
            Self::Assignment => "assignment",
            Self::ImportUsage => "import_usage",
            Self::Block => "block",
            Self::Unknown => "unknown",
        };
        write!(f, "{}", name)
    }
}
