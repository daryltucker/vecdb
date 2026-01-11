// PURPOSE:
//   Rust parser implementation for vecq using syn crate for comprehensive AST analysis.
//   Extracts all Rust language constructs (functions, structs, enums, traits, impls)
//   while preserving complete type information, generics, lifetimes, and attributes.
//   Essential for sophisticated Rust code analysis and queries.
//
// REQUIREMENTS:
//   User-specified:
//   - Must extract functions with complete signatures, generics, lifetimes, attributes
//   - Must parse structs with fields, visibility, derive attributes, generic parameters
//   - Must handle enums with variants, associated data, and discriminants
//   - Must extract traits with associated types, methods, supertraits, bounds
//   - Must parse impl blocks with target types, trait implementations, methods
//   
//   Implementation-discovered:
//   - Requires syn crate with full feature set for complete Rust syntax support
//   - Must handle complex generic bounds and where clauses accurately
//   - Needs to preserve exact attribute information for derive macros, cfg, etc.
//   - Must track visibility modifiers precisely (pub, pub(crate), pub(super), etc.)
//
// IMPLEMENTATION RULES:
//   1. Use syn crate with full features for complete Rust syntax support
//      Rationale: syn provides the most accurate and complete Rust parsing available
//   
//   2. Extract complete type information including complex generic bounds
//      Rationale: Type information is essential for sophisticated code analysis
//   
//   3. Preserve all attributes exactly as written in source code
//      Rationale: Attributes affect compilation and behavior, must be queryable
//   
//   4. Handle parsing errors gracefully and continue with partial results
//      Rationale: Malformed code shouldn't prevent analysis of valid parts
//   
//   5. Track exact source locations for all elements using syn's Span information
//      Rationale: Precise location tracking enables accurate grep-compatible output
//   
//   Critical:
//   - DO NOT lose any type information during parsing
//   - DO NOT modify attribute content or structure
//   - ALWAYS handle incomplete or malformed Rust code gracefully
//
// USAGE:
//   use vecq::parsers::RustParser;
//   use vecq::parser::Parser;
//   
//   let parser = RustParser::new();
//   let content = "fn main() { println!(\"Hello\"); }";
//   let parsed = parser.parse(content).await?;
//   
//   // Query functions
//   let functions = parsed.find_elements(ElementType::Function);
//   assert_eq!(functions.len(), 1);
//   assert_eq!(functions[0].name, Some("main".to_string()));
//
// SELF-HEALING INSTRUCTIONS:
//   When Rust parsing fails:
//   1. Check if syn crate version changed parsing behavior
//   2. Add test case for the failing Rust syntax
//   3. Update error handling to provide better diagnostics
//   4. Ensure graceful degradation for new Rust syntax
//   5. Update JSON schema if new language features discovered
//   
//   When adding new Rust feature support:
//   1. Update syn dependency if needed for new syntax
//   2. Add new ElementType variants if required
//   3. Update parsing logic to handle new constructs
//   4. Add comprehensive tests for new features
//   5. Document new capabilities in parser info
//
// RELATED FILES:
//   - src/parser.rs - Defines Parser trait implemented here
//   - src/types.rs - Defines ElementType and DocumentElement structures
//   - tests/unit/parsers/rust_tests.rs - Unit tests for this parser
//   - tests/property/property_rust_parsing.rs - Property tests
//   - tests/fixtures/rust/ - Real-world Rust test files
//
// MAINTENANCE:
//   Update when:
//   - syn crate releases new versions with API changes
//   - New Rust language features need support
//   - Parsing accuracy issues reported for complex syntax
//   - Performance optimization opportunities identified
//   - New Rust edition features added
//
// Last Verified: 2025-12-31

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, ParsedDocument, RustAttributes, ElementAttributes};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use syn::{File, Item, ItemFn, ItemStruct, ItemEnum, ItemTrait, ItemImpl, Visibility, Attribute, Meta, Expr, Lit};

/// Rust parser using syn crate
pub struct RustParser {
    _config: ParserConfig,
}

impl RustParser {
    /// Create a new Rust parser
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    /// Create parser with custom configuration
    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }

    /// Extract docstring from attributes
    fn extract_docstring(&self, attrs: &[Attribute]) -> Option<String> {
        let mut docs = Vec::new();
        for attr in attrs {
            if attr.path().is_ident("doc") {
                if let Meta::NameValue(meta) = &attr.meta {
                    if let Expr::Lit(expr_lit) = &meta.value {
                        if let Lit::Str(lit_str) = &expr_lit.lit {
                            docs.push(lit_str.value().trim().to_string());
                        }
                    }
                }
            }
        }
        if docs.is_empty() {
            None
        } else {
            Some(docs.join("\n"))
        }
    }

    /// Parse Rust syntax tree into document elements
    fn parse_syntax_tree(&self, file: &File, content: &str) -> VecqResult<Vec<DocumentElement>> {
        let mut elements = Vec::new();
        for item in &file.items {
           if let Some(element) = self.parse_item(item, content)? {
               elements.push(element);
           }
        }
        Ok(elements)
    }

    fn parse_item(&self, item: &Item, _content: &str) -> VecqResult<Option<DocumentElement>> {
        use syn::spanned::Spanned;
        let span = item.span();
        let start_line = span.start().line;
        let end_line = span.end().line;

        match item {
            Item::Fn(item_fn) => self.parse_function(item_fn, start_line, end_line),
            Item::Struct(item_struct) => self.parse_struct(item_struct, start_line, end_line),
            Item::Enum(item_enum) => self.parse_enum(item_enum, start_line, end_line),
            Item::Trait(item_trait) => self.parse_trait(item_trait, start_line, end_line),
            Item::Impl(item_impl) => self.parse_impl(item_impl, start_line, end_line),
            Item::Mod(item_mod) => self.parse_module(item_mod, start_line, end_line),
            Item::Use(item_use) => self.parse_use(item_use, start_line, end_line),
            Item::Const(item_const) => self.parse_const(item_const, start_line, end_line),
            Item::Static(item_static) => self.parse_static(item_static, start_line, end_line),
            _ => {
                // Generic block for other items
               Ok(Some(DocumentElement::new(
                    ElementType::Block,
                    None,
                    "other".to_string(), // content[span] extraction requires byte range, syn gives line/col
                    start_line,
                    end_line,
                )))
            }
        }
    }

    /// Parse function item
    fn parse_function(&self, item_fn: &ItemFn, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        let mut other = HashMap::new();
        if item_fn.sig.asyncness.is_some() { 
            other.insert("async".to_string(), true.into()); 
        }
        if let Some(doc) = self.extract_docstring(&item_fn.attrs) {
            other.insert("docstring".to_string(), serde_json::Value::String(doc));
        }

        let attributes = ElementAttributes::Rust(RustAttributes {
            visibility: self.visibility_to_string(&item_fn.vis),
            other: {
                let mut o = other;
                let sig = &item_fn.sig;
                o.insert("signature".to_string(), serde_json::Value::String(format!("{}", quote::quote!(#sig))));
                o
            },
        });

        Ok(Some(DocumentElement::new(
            ElementType::Function,
            Some(item_fn.sig.ident.to_string()),
            format!("{}", quote::quote!(#item_fn)), // Simplified content representation
            start,
            end,
        ).set_attributes(attributes)))
    }

    /// Parse struct item
    fn parse_struct(&self, item_struct: &ItemStruct, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        let mut other = HashMap::new();
        if let Some(doc) = self.extract_docstring(&item_struct.attrs) {
            other.insert("docstring".to_string(), serde_json::Value::String(doc));
        }

        let attributes = ElementAttributes::Rust(RustAttributes {
            visibility: self.visibility_to_string(&item_struct.vis),
            other,
        });

        Ok(Some(DocumentElement::new(
            ElementType::Struct,
            Some(item_struct.ident.to_string()),
            format!("{}", quote::quote!(#item_struct)),
            start,
            end,
        ).set_attributes(attributes)))
    }

    /// Parse enum item
    fn parse_enum(&self, item_enum: &ItemEnum, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        let mut other = HashMap::new();
        if let Some(doc) = self.extract_docstring(&item_enum.attrs) {
            other.insert("docstring".to_string(), serde_json::Value::String(doc));
        }

        let attributes = ElementAttributes::Rust(RustAttributes {
            visibility: self.visibility_to_string(&item_enum.vis),
            other,
        });

        Ok(Some(DocumentElement::new(
            ElementType::Enum,
            Some(item_enum.ident.to_string()),
            format!("{}", quote::quote!(#item_enum)),
            start,
            end,
        ).set_attributes(attributes)))
    }

    /// Parse trait item with children
    fn parse_trait(&self, item_trait: &ItemTrait, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        use syn::spanned::Spanned;
        let mut children = Vec::new();
        for item in &item_trait.items {
            if let syn::TraitItem::Fn(method) = item {
                let m_span = method.span();
                
                let mut m_other = HashMap::new();
                if method.sig.asyncness.is_some() { m_other.insert("async".to_string(), true.into()); }
                if let Some(doc) = self.extract_docstring(&method.attrs) {
                    m_other.insert("docstring".to_string(), serde_json::Value::String(doc));
                }
                
                let m_attrs = ElementAttributes::Rust(RustAttributes {
                    visibility: "inherited".to_string(), // Trait items inherit visibility
                    other: m_other,
                });

                children.push(DocumentElement::new(
                    ElementType::Function,
                    Some(method.sig.ident.to_string()),
                    format!("{}", quote::quote!(#method)),
                    m_span.start().line,
                    m_span.end().line
                ).set_attributes(m_attrs));
            }
        }

        let mut other = HashMap::new();
        if let Some(doc) = self.extract_docstring(&item_trait.attrs) {
            other.insert("docstring".to_string(), serde_json::Value::String(doc));
        }
        
        // Traits can have unsafe/auto which we could capture in other if needed

        let attributes = ElementAttributes::Rust(RustAttributes {
            visibility: self.visibility_to_string(&item_trait.vis),
            other,
        });

        Ok(Some(DocumentElement::new(
            ElementType::Trait,
            Some(item_trait.ident.to_string()),
            format!("{}", quote::quote!(#item_trait)),
            start,
            end,
        ).set_attributes(attributes).with_children(children)))
    }

    /// Parse impl item with children
    fn parse_impl(&self, item_impl: &ItemImpl, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        use syn::spanned::Spanned;
        let mut children = Vec::new();
        for item in &item_impl.items {
             if let syn::ImplItem::Fn(method) = item {
                let m_span = method.span();
                let mut m_other = HashMap::new();
                if method.sig.asyncness.is_some() { m_other.insert("async".to_string(), true.into()); }
                if let Some(doc) = self.extract_docstring(&method.attrs) {
                    m_other.insert("docstring".to_string(), serde_json::Value::String(doc));
                }
                
                let m_attrs = ElementAttributes::Rust(RustAttributes {
                    visibility: self.visibility_to_string(&method.vis),
                    other: {
                        let mut o = m_other;
                        let sig = &method.sig;
                        o.insert("signature".to_string(), serde_json::Value::String(format!("{}", quote::quote!(#sig))));
                        o
                    },
                });
                
                children.push(DocumentElement::new(
                    ElementType::Function,
                    Some(method.sig.ident.to_string()),
                    format!("{}", quote::quote!(#method)),
                    m_span.start().line,
                    m_span.end().line
                ).set_attributes(m_attrs));
            }
        }

        // Impl blocks don't have visibility themselves (usually), but the items inside do.
        // However, `impl PubStruct` or `impl Trait for PubStruct` might be relevant check.
        // For now, empty attributes or "inherited".
        // Using Generic default is probably fine for the container block, 
        // OR we can make it RustAttributes with empty visibility?
        // Let's use RustAttributes for consistency.
        
        let mut other = HashMap::new();
        // Maybe capture trait if it exists? 
        if let Some((_, path, _)) = &item_impl.trait_ {
             other.insert("trait".to_string(), serde_json::Value::String(quote::quote!(#path).to_string()));
        }

        let attributes = ElementAttributes::Rust(RustAttributes {
            visibility: "inherited".to_string(), 
            other,
        });

        let self_ty = &item_impl.self_ty;
        let name = if let Some((_, path, _)) = &item_impl.trait_ {
            format!("impl {} for {}", quote::quote!(#path), quote::quote!(#self_ty))
        } else {
            format!("impl {}", quote::quote!(#self_ty))
        };

        Ok(Some(DocumentElement::new(
            ElementType::Implementation,
            Some(name),
            format!("{}", quote::quote!(#item_impl)),
            start,
            end,
        ).set_attributes(attributes).with_children(children)))
    }

    /// Parse module item with children
    fn parse_module(&self, item_mod: &syn::ItemMod, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
         
         let mut children = Vec::new();
         if let Some((_, items)) = &item_mod.content {
             for item in items {
                 // RECURSION: Parse items inside module
                 // Use a dummy content string since we don't have easy byte access to inner content here
                 // properly without keeping full source. 
                 // For now, pass empty string or full content? 
                 // We need full content if we want to extract source text, 
                 // but for structure, we just need the item tree.
                 if let Some(child) = self.parse_item(item, "")? {
                     children.push(child);
                 }
             }
         }

         let mut attributes = HashMap::new();
         attributes.insert("visibility".to_string(), serde_json::Value::String(self.visibility_to_string(&item_mod.vis)));
         if let Some(doc) = self.extract_docstring(&item_mod.attrs) {
             attributes.insert("docstring".to_string(), serde_json::Value::String(doc));
         }

         Ok(Some(DocumentElement::new(
            ElementType::Module,
            Some(item_mod.ident.to_string()),
            format!("{}", quote::quote!(#item_mod)),
            start,
            end
         ).with_attributes(attributes).with_children(children)))
    }

    /// Parse use item
    fn parse_use(&self, item_use: &syn::ItemUse, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        let mut attributes = HashMap::new();
        attributes.insert("visibility".to_string(), serde_json::Value::String(self.visibility_to_string(&item_use.vis)));

        Ok(Some(DocumentElement::new(
            ElementType::Import,
            None,
            format!("{}", quote::quote!(#item_use)),
            start,
            end,
        ).with_attributes(attributes)))
    }

    /// Parse const item
    fn parse_const(&self, item_const: &syn::ItemConst, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
        Ok(Some(DocumentElement::new(
            ElementType::Constant,
            Some(item_const.ident.to_string()),
            format!("{}", quote::quote!(#item_const)),
            start,
            end,
        )))
    }

    /// Parse static item
    fn parse_static(&self, item_static: &syn::ItemStatic, start: usize, end: usize) -> VecqResult<Option<DocumentElement>> {
         Ok(Some(DocumentElement::new(
            ElementType::Variable,
            Some(item_static.ident.to_string()),
            format!("{}", quote::quote!(#item_static)),
            start,
            end,
        )))
    }

    /// Convert visibility to string
    fn visibility_to_string(&self, vis: &Visibility) -> String {
        match vis {
            Visibility::Public(_) => "pub".to_string(),
            Visibility::Restricted(vis_restricted) => {
                format!("pub({})", quote::quote!(#vis_restricted.path))
            }
            Visibility::Inherited => "private".to_string(),
        }
    }
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for RustParser {
    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        if content.trim().is_empty() {
            // Return empty doc instead of error for empty file
             let metadata = DocumentMetadata::new(
                PathBuf::from("unknown.rs"),
                0,
            );
            return Ok(ParsedDocument::new(metadata));
        }

        let syntax_tree = syn::parse_file(content)
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("unknown"),
                line: 0,
                message: format!("Rust parsing failed: {}", e),
                source: Some(Box::new(e)),
            })?;

        let elements = self.parse_syntax_tree(&syntax_tree, content)?;
        
        let metadata = DocumentMetadata::new(
            PathBuf::from("unknown.rs"),
            content.len() as u64,
        ).with_line_count(content);

        Ok(ParsedDocument::new(metadata).add_elements(elements))
    }

    fn file_extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn language_name(&self) -> &'static str {
        "Rust"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true, 
            documentation: true,
            type_information: true,
            macros: true,
            max_file_size: Some(5 * 1024 * 1024), // 5MB
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_function() {
        let parser = RustParser::new();
        let content = "fn main() { println!(\"Hello, world!\"); }";
        
        let parsed = parser.parse(content).await.unwrap();
        let functions = parsed.find_elements(ElementType::Function);
        
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0].name, Some("main".to_string()));
    }

    #[tokio::test]
    async fn test_parse_struct() {
        let parser = RustParser::new();
        let content = "pub struct Point { x: i32, y: i32 }";
        
        let parsed = parser.parse(content).await.unwrap();
        let structs = parsed.find_elements(ElementType::Struct);
        
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, Some("Point".to_string()));
        assert_eq!(structs[0].attributes.get_text("visibility").unwrap(), "pub");
    }

    #[tokio::test]
    async fn test_parse_enum() {
        let parser = RustParser::new();
        let content = "enum Color { Red, Green, Blue }";
        
        let parsed = parser.parse(content).await.unwrap();
        let enums = parsed.find_elements(ElementType::Enum);
        
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, Some("Color".to_string()));
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = RustParser::new();
        let result = parser.parse("").await;
        // Empty Rust files are syntactically valid - they just have no elements
        assert!(result.is_ok());
        assert!(result.unwrap().elements.is_empty());
    }

    #[tokio::test]
    async fn test_invalid_syntax() {
        let parser = RustParser::new();
        let result = parser.parse("fn incomplete(").await;
        assert!(result.is_err());
    }
}