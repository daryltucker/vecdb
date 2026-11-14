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
    HtmlBlock,   // Markdown: a raw HTML block (`<details>`, `<!-- -->`, ...)
    FrontMatter, // Markdown: leading YAML `---` or TOML `+++` metadata block
    Definition,  // Markdown: link reference definition `[label]: url "title"` (MDAST `Definition`)

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
            Self::HtmlBlock => "html_block",
            Self::FrontMatter => "front_matter",
            Self::Definition => "definition",
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

impl ElementType {
    /// The output key an element of this type is listed under when its
    /// language's schema names no other. Always the plural of a list:
    /// `.functions[]`, `.variables[]`, `.blocks[]`.
    ///
    /// The fallback used to be the singular type name
    /// (`Display`), so a language whose schema forgot a mapping listed the
    /// same kind under a second spelling — Python/C/C++/Go `variable` beside
    /// Bash/JavaScript `variables`, C++/CUDA `struct`/`macro` beside C's
    /// `structs`/`macros`. A query over a mixed tree silently missed half of
    /// them. Exhaustive on purpose: a new variant does not compile without its
    /// key. Mass nouns (`emphasis`, `strong`, …) and names that already
    /// shipped (`footnotes`, `elements`, `package`) match their schema
    /// mappings so the fallback never introduces a second spelling.
    pub fn category_key(self) -> &'static str {
        match self {
            Self::Function => "functions",
            Self::Class => "classes",
            Self::Struct => "structs",
            Self::Enum => "enums",
            Self::Union => "unions",
            Self::Interface => "interfaces",
            Self::Module => "modules",
            Self::Import => "imports",
            Self::Variable => "variables",
            Self::Constant => "constants",
            Self::Comment => "comments",
            Self::Header => "headers",
            Self::CodeBlock => "code_blocks",
            Self::Link => "links",
            Self::Table => "tables",
            Self::List => "lists",
            Self::ListItem => "list_items",
            Self::Blockquote => "blockquotes",
            Self::Paragraph => "paragraphs",
            Self::HorizontalRule => "horizontal_rules",
            Self::Image => "images",
            Self::Emphasis => "emphasis",
            Self::Strong => "strong",
            Self::Strikethrough => "strikethrough",
            Self::FootnoteDefinition => "footnotes",
            Self::HtmlElement => "elements",
            Self::HtmlBlock => "html_blocks",
            Self::FrontMatter => "front_matter",
            Self::Definition => "definitions",
            Self::Trait => "traits",
            Self::Implementation => "implementations",
            Self::Decorator => "decorators",
            Self::Macro => "macros",
            Self::Namespace => "namespaces",
            Self::Package => "package",
            Self::Kernel => "kernels",
            Self::DeviceFunction => "device_functions",
            Self::TypeAlias => "type_aliases",
            Self::FunctionCall => "function_calls",
            Self::VariableReference => "variable_references",
            Self::TypeReference => "type_references",
            Self::MethodCall => "method_calls",
            Self::Assignment => "assignments",
            Self::ImportUsage => "import_usages",
            Self::Block => "blocks",
            Self::Unknown => "unknown",
        }
    }
}
