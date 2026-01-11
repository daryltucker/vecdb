// PURPOSE:
//   JSON conversion engine that transforms parsed documents into consistent JSON schemas.
//   Critical for vecq's core value proposition - making any structured document queryable
//   with jq syntax. Ensures schema consistency across all file types while preserving
//   all structural information and metadata for accurate querying.
//
// REQUIREMENTS:
//   User-specified:
//   - Must maintain consistent JSON schema patterns across different file types
//   - Must preserve all structural information including line numbers and metadata
//   - Must support hierarchical document structures with nested elements
//   - Must enable jq-compatible querying of all document types
//   
//   Implementation-discovered:
//   - Requires serde_json for JSON serialization and manipulation
//   - Must handle large documents efficiently without memory issues
//   - Needs versioning support for schema evolution
//   - Must support custom field mappings for language-specific attributes
//
// IMPLEMENTATION RULES:
//   1. All JSON outputs must include common metadata fields (file_type, path, line_count)
//      Rationale: Enables consistent querying patterns across all document types
//   
//   2. Use snake_case for all JSON field names consistently
//      Rationale: Matches jq conventions and provides predictable query syntax
//   
//   3. Preserve exact line numbers for all structural elements
//      Rationale: Required for grep compatibility and source location tracking
//   
//   4. Use consistent element type names across languages where possible
//      Rationale: Enables cross-language queries like "find all functions"
//   
//   5. Schema registry must support versioning for backward compatibility
//      Rationale: Allows schema evolution without breaking existing queries
//   
//   Critical:
//   - DO NOT change existing JSON field names without migration plan
//   - DO NOT lose structural information during conversion
//   - ALWAYS preserve hierarchical relationships between elements
//
// USAGE:
//   use vecq::converter::{JsonConverter, UnifiedJsonConverter, SchemaRegistry};
//   use vecq::types::{ParsedDocument, FileType};
//   
//   // Create converter with schema registry
//   let mut registry = SchemaRegistry::new();
//   let converter = UnifiedJsonConverter::new(registry);
//   
//   // Convert document to JSON
//   let json_value = converter.convert(parsed_document)?;
//   
//   // Query with jq syntax
//   let functions = jq::run(".functions[] | .name", &json_value)?;
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new file type support:
//   1. Add schema definition to SchemaRegistry for new file type
//   2. Update convert() method to handle new DocumentElement types
//   3. Ensure consistent field naming with existing schemas
//   4. Add JSON schema validation tests
//   5. Update property tests to include new file type
//   6. Document new schema in design document
//   
//   When modifying existing schemas:
//   1. Increment schema version number
//   2. Add backward compatibility handling for old versions
//   3. Update all related tests and documentation
//   4. Provide migration guide for existing users
//   5. Test with real-world fixtures to ensure compatibility
//
// RELATED FILES:
//   - src/types.rs - Defines ParsedDocument and DocumentElement structures
//   - src/parser.rs - Produces ParsedDocument that gets converted to JSON
//   - src/query.rs - Consumes JSON output for jq querying
//   - src/parsers/*.rs - Language parsers that create elements for conversion
//   - tests/unit/converter_tests.rs - JSON conversion validation
//
// MAINTENANCE:
//   Update when:
//   - New file types are added with unique structural elements
//   - JSON schema needs evolution for new query patterns
//   - Performance optimization requires schema changes
//   - jq compatibility requires field name adjustments
//   - User feedback indicates schema improvements needed
//
// Last Verified: 2025-12-31

use crate::error::{VecqError, VecqResult};
use crate::types::{DocumentElement, ElementType, FileType, ParsedDocument};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Trait for converting parsed documents to JSON
pub trait JsonConverter: Send + Sync {
    /// Convert a parsed document to JSON representation
    fn convert(&self, document: ParsedDocument) -> VecqResult<Value>;

    /// Get the schema version used by this converter
    fn schema_version(&self) -> &str;

    /// Validate that a JSON value conforms to expected schema
    fn validate_schema(&self, json: &Value, file_type: FileType) -> VecqResult<()>;
}

/// Schema definition for a specific file type
#[derive(Debug, Clone)]
pub struct Schema {
    pub version: String,
    pub file_type: FileType,
    pub required_fields: Vec<String>,
    pub element_mappings: HashMap<ElementType, String>,
}

impl Schema {
    /// Create a new schema definition
    pub fn new(version: String, file_type: FileType) -> Self {
        Self {
            version,
            file_type,
            required_fields: vec![
                "file_type".to_string(),
                "metadata".to_string(),
                "elements".to_string(),
            ],
            element_mappings: HashMap::new(),
        }
    }

    /// Add required field to schema
    pub fn with_required_field(mut self, field: String) -> Self {
        self.required_fields.push(field);
        self
    }

    /// Add element type mapping
    pub fn with_element_mapping(mut self, element_type: ElementType, json_field: String) -> Self {
        self.element_mappings.insert(element_type, json_field);
        self
    }

    /// Get JSON field name for element type
    pub fn get_element_field(&self, element_type: ElementType) -> String {
        self.element_mappings
            .get(&element_type)
            .cloned()
            .unwrap_or_else(|| element_type.to_string())
    }
}

/// Registry for managing JSON schemas for different file types
#[derive(Default)]
pub struct SchemaRegistry {
    schemas: HashMap<FileType, Schema>,
}

impl SchemaRegistry {
    /// Create a new schema registry with default schemas
    pub fn new() -> Self {
        let mut registry = Self::default();
        registry.register_default_schemas();
        registry
    }

    /// Register a schema for a file type
    pub fn register(&mut self, schema: Schema) {
        self.schemas.insert(schema.file_type, schema);
    }

    /// Get schema for a file type
    pub fn get_schema(&self, file_type: FileType) -> VecqResult<&Schema> {
        self.schemas
            .get(&file_type)
            .ok_or_else(|| VecqError::UnsupportedFileType {
                file_type: file_type.to_string(),
            })
    }

    /// Register default schemas for all supported file types
    fn register_default_schemas(&mut self) {
        // Rust schema
        let rust_schema = Schema::new("1.0".to_string(), FileType::Rust)
            .with_required_field("functions".to_string())
            .with_required_field("structs".to_string())
            .with_required_field("enums".to_string())
            .with_element_mapping(ElementType::Function, "functions".to_string())
            .with_element_mapping(ElementType::Struct, "structs".to_string())
            .with_element_mapping(ElementType::Enum, "enums".to_string())
            .with_element_mapping(ElementType::Trait, "traits".to_string())
            .with_element_mapping(ElementType::Implementation, "implementations".to_string())
            .with_element_mapping(ElementType::Import, "use_statements".to_string());
        self.register(rust_schema);

        // Python schema
        let python_schema = Schema::new("1.0".to_string(), FileType::Python)
            .with_required_field("classes".to_string())
            .with_required_field("functions".to_string())
            .with_required_field("imports".to_string())
            .with_element_mapping(ElementType::Class, "classes".to_string())
            .with_element_mapping(ElementType::Function, "functions".to_string())
            .with_element_mapping(ElementType::Import, "imports".to_string())
            .with_element_mapping(ElementType::Decorator, "decorators".to_string());
        self.register(python_schema);

        // Markdown schema
        let markdown_schema = Schema::new("1.0".to_string(), FileType::Markdown)
            .with_required_field("headers".to_string())
            .with_required_field("code_blocks".to_string())
            .with_required_field("links".to_string())
            .with_element_mapping(ElementType::Header, "headers".to_string())
            .with_element_mapping(ElementType::CodeBlock, "code_blocks".to_string())
            .with_element_mapping(ElementType::Link, "links".to_string())
            .with_element_mapping(ElementType::Table, "tables".to_string())
            .with_element_mapping(ElementType::List, "lists".to_string());
        self.register(markdown_schema);

        // Add schemas for other file types
        self.register_c_cpp_schemas();
        self.register_cuda_schema();
        self.register_go_schema();
        self.register_bash_schema();
        self.register_bash_schema();
        self.register_text_schema();
        self.register_toml_schema();
        self.register_html_schema();
        self.register_json_schema();
    }


    fn register_c_cpp_schemas(&mut self) {
        // C schema
        let c_schema = Schema::new("1.0".to_string(), FileType::C)
            .with_required_field("functions".to_string())
            .with_required_field("structs".to_string())
            .with_element_mapping(ElementType::Function, "functions".to_string())
            .with_element_mapping(ElementType::Struct, "structs".to_string())
            .with_element_mapping(ElementType::Macro, "macros".to_string());
        self.register(c_schema);

        // C++ schema
        let cpp_schema = Schema::new("1.0".to_string(), FileType::Cpp)
            .with_required_field("functions".to_string())
            .with_required_field("classes".to_string())
            .with_required_field("namespaces".to_string())
            .with_element_mapping(ElementType::Function, "functions".to_string())
            .with_element_mapping(ElementType::Class, "classes".to_string())
            .with_element_mapping(ElementType::Namespace, "namespaces".to_string());
        self.register(cpp_schema);
    }

    fn register_cuda_schema(&mut self) {
        let cuda_schema = Schema::new("1.0".to_string(), FileType::Cuda)
            .with_required_field("kernels".to_string())
            .with_required_field("device_functions".to_string())
            .with_required_field("host_functions".to_string())
            .with_element_mapping(ElementType::Kernel, "kernels".to_string())
            .with_element_mapping(ElementType::DeviceFunction, "device_functions".to_string())
            .with_element_mapping(ElementType::Function, "host_functions".to_string());
        self.register(cuda_schema);
    }

    fn register_go_schema(&mut self) {
        let go_schema = Schema::new("1.0".to_string(), FileType::Go)
            .with_required_field("functions".to_string())
            .with_required_field("structs".to_string())
            .with_required_field("interfaces".to_string())
            .with_element_mapping(ElementType::Function, "functions".to_string())
            .with_element_mapping(ElementType::Struct, "structs".to_string())
            .with_element_mapping(ElementType::Interface, "interfaces".to_string())
            .with_element_mapping(ElementType::Package, "package".to_string());
        self.register(go_schema);
    }

    fn register_bash_schema(&mut self) {
        let bash_schema = Schema::new("1.0".to_string(), FileType::Bash)
            .with_required_field("functions".to_string())
            .with_required_field("variables".to_string())
            .with_element_mapping(ElementType::Function, "functions".to_string())
            .with_element_mapping(ElementType::Variable, "variables".to_string());
        self.register(bash_schema);
    }

    fn register_text_schema(&mut self) {
        let text_schema = Schema::new("1.0".to_string(), FileType::Text)
            .with_required_field("blocks".to_string())
            .with_element_mapping(ElementType::Block, "blocks".to_string());
        self.register(text_schema);
    }

    fn register_toml_schema(&mut self) {
        let toml_schema = Schema::new("1.0".to_string(), FileType::Toml)
            .with_required_field("tables".to_string())
            .with_required_field("entries".to_string())
            .with_element_mapping(ElementType::Block, "tables".to_string())
            .with_element_mapping(ElementType::Variable, "entries".to_string());
        self.register(toml_schema);
    }

    fn register_html_schema(&mut self) {
        let html_schema = Schema::new("1.0".to_string(), FileType::Html)
            .with_required_field("elements".to_string())
            .with_element_mapping(ElementType::HtmlElement, "elements".to_string());
        self.register(html_schema);
    }

    fn register_json_schema(&mut self) {
        let json_schema = Schema::new("1.0".to_string(), FileType::Json)
            .with_required_field("metadata".to_string());
        self.register(json_schema);
    }
}


/// Unified JSON converter that handles all file types
pub struct UnifiedJsonConverter {
    schema_registry: SchemaRegistry,
    context_lines: usize,
}

impl UnifiedJsonConverter {
    /// Create a new unified JSON converter
    pub fn new(schema_registry: SchemaRegistry) -> Self {
        Self { 
            schema_registry,
            context_lines: 0, 
        }
    }

    /// Set number of context lines to include
    pub fn with_context_lines(mut self, lines: usize) -> Self {
        self.context_lines = lines;
        self
    }

    /// Create converter with default schemas
    pub fn with_default_schemas() -> Self {
        Self::new(SchemaRegistry::new())
    }

    /// Convert document element to JSON value
    fn convert_element(&self, element: &DocumentElement, crumbtrail: &str, doc: &ParsedDocument) -> Value {
        let mut obj = Map::new();

        // Basic element information
        obj.insert("type".to_string(), json!(element.element_type.to_string()));
        if let Some(ref name) = element.name {
            obj.insert("name".to_string(), json!(name));
        }
        obj.insert("content".to_string(), json!(element.content));
        obj.insert("line_start".to_string(), json!(element.line_start));
        obj.insert("line_end".to_string(), json!(element.line_end));
        
        if !crumbtrail.is_empty() {
            obj.insert("crumbtrail".to_string(), json!(crumbtrail));
        }

        // Context lines
        if self.context_lines > 0 {
            let context_before = doc.get_context_before(element.line_start, self.context_lines);
            if !context_before.is_empty() {
                obj.insert("context_before".to_string(), json!(context_before));
            }
            
            let context_after = doc.get_context_after(element.line_end, self.context_lines);
            if !context_after.is_empty() {
                obj.insert("context_after".to_string(), json!(context_after));
            }
        }

        // Attributes
        if !element.attributes.is_empty() {
            obj.insert("attributes".to_string(), json!(element.attributes));
        }

        // Children handled by group_elements_by_type for now to avoid double recursion complexity here
        // or we can add them here if needed. 

        Value::Object(obj)
    }

    /// Group elements by type for structured output
    fn group_elements_by_type(&self, elements: &[DocumentElement], parent_path: String, doc: &ParsedDocument) -> HashMap<String, Vec<Value>> {
        let mut grouped: HashMap<String, Vec<Value>> = HashMap::new();

        for element in elements {
            // Use schema for the document's file type
            let field_name = self.schema_registry.get_schema(doc.file_type())
                .unwrap_or_else(|_| self.schema_registry.get_schema(FileType::Text).unwrap()) // Fallback
                .get_element_field(element.element_type);
            
            // Build current path
            let current_path = if let Some(name) = &element.name {
                if parent_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", parent_path, name)
                }
            } else {
                parent_path.clone()
            };

            let mut json_element = self.convert_element(element, &parent_path, doc);
            
            // Recursively process children
            if !element.children.is_empty() {
                let children_grouped = self.group_elements_by_type(&element.children, current_path.clone(), doc);
                
                // Merge children's flattened lists into the main grouped list
                for (k, mut v) in children_grouped {
                    grouped.entry(k).or_default().append(&mut v);
                }
            }

            // check if json_element is an object (it should be)
            if let Value::Object(ref mut map) = json_element {
                 if !element.children.is_empty() {
                     let mut children_arr = Vec::new();
                     for child in &element.children {
                         children_arr.push(self.convert_element(child, &current_path, doc));
                     }
                     map.insert("children".to_string(), Value::Array(children_arr));
                 }
            }

            grouped
                .entry(field_name)
                .or_default()
                .push(json_element);
        }

        grouped
    }

    /// Create metadata JSON object
    fn create_metadata_json(&self, document: &ParsedDocument) -> Value {
        let metadata = &document.metadata;
        json!({
            "file_type": metadata.file_type.to_string(),
            "path": metadata.path.to_string_lossy(),
            "size": metadata.size,
            "modified": metadata.modified,
            "encoding": metadata.encoding,
            "line_count": metadata.line_count,
            "hash": metadata.hash
        })
    }
}

impl JsonConverter for UnifiedJsonConverter {
    fn convert(&self, document: ParsedDocument) -> VecqResult<Value> {
        let schema = self.schema_registry.get_schema(document.file_type())?;
        
        let mut result = Map::new();

        // Add metadata
        result.insert("metadata".to_string(), self.create_metadata_json(&document));

        // Group elements by type according to schema - pass empty string for root
        let grouped_elements = self.group_elements_by_type(&document.elements, String::new(), &document);
        
        // Add grouped elements to result
        for (field_name, elements) in grouped_elements {
            result.insert(field_name, Value::Array(elements));
        }

        // Ensure all required fields are present (empty arrays if no elements)
        for required_field in &schema.required_fields {
            if !result.contains_key(required_field) {
                result.insert(required_field.clone(), json!([]));
            }
        }

        Ok(Value::Object(result))
    }

    fn schema_version(&self) -> &str {
        "1.0"
    }

    fn validate_schema(&self, json: &Value, file_type: FileType) -> VecqResult<()> {
        let schema = self.schema_registry.get_schema(file_type)?;
        
        let obj = json.as_object().ok_or_else(|| {
            VecqError::json_error("JSON must be an object".to_string(), None::<std::io::Error>)
        })?;

        // Check required fields
        for required_field in &schema.required_fields {
            if !obj.contains_key(required_field) {
                return Err(VecqError::json_error(
                    format!("Missing required field: {}", required_field),
                    None::<std::io::Error>,
                ));
            }
        }

        // Validate metadata structure
        if let Some(metadata) = obj.get("metadata") {
            let metadata_obj = metadata.as_object().ok_or_else(|| {
                VecqError::json_error("Metadata must be an object".to_string(), None::<std::io::Error>)
            })?;

            let required_metadata_fields = ["file_type", "path", "line_count"];
            for field in &required_metadata_fields {
                if !metadata_obj.contains_key(*field) {
                    return Err(VecqError::json_error(
                        format!("Missing required metadata field: {}", field),
                        None::<std::io::Error>,
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DocumentMetadata, ElementType};
    use std::path::PathBuf;

    fn create_test_document() -> ParsedDocument {
        let metadata = DocumentMetadata::new(PathBuf::from("test.rs"), 100)
            .with_line_count("fn main() {}\nstruct Test {}");

        let function_element = DocumentElement::new(
            ElementType::Function,
            Some("main".to_string()),
            "fn main() {}".to_string(),
            1,
            1,
        );

        let struct_element = DocumentElement::new(
            ElementType::Struct,
            Some("Test".to_string()),
            "struct Test {}".to_string(),
            2,
            2,
        );

        ParsedDocument::new(metadata)
            .add_element(function_element)
            .add_element(struct_element)
    }

    #[test]
    fn test_schema_registry() {
        let registry = SchemaRegistry::new();
        
        let rust_schema = registry.get_schema(FileType::Rust).unwrap();
        assert_eq!(rust_schema.version, "1.0");
        assert_eq!(rust_schema.file_type, FileType::Rust);
        assert!(rust_schema.required_fields.contains(&"functions".to_string()));
    }

    #[test]
    fn test_json_conversion() {
        let converter = UnifiedJsonConverter::with_default_schemas();
        let document = create_test_document();
        
        let json = converter.convert(document).unwrap();
        
        // Check structure
        assert!(json.is_object());
        let obj = json.as_object().unwrap();
        
        assert!(obj.contains_key("metadata"));
        assert!(obj.contains_key("functions"));
        assert!(obj.contains_key("structs"));
        
        // Check functions array
        let functions = obj.get("functions").unwrap().as_array().unwrap();
        assert_eq!(functions.len(), 1);
        assert_eq!(functions[0]["name"], "main");
        assert_eq!(functions[0]["line_start"], 1);
        
        // Check structs array
        let structs = obj.get("structs").unwrap().as_array().unwrap();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0]["name"], "Test");
        assert_eq!(structs[0]["line_start"], 2);
    }

    #[test]
    fn test_schema_validation() {
        let converter = UnifiedJsonConverter::with_default_schemas();
        let document = create_test_document();
        let json = converter.convert(document).unwrap();
        
        // Valid JSON should pass validation
        assert!(converter.validate_schema(&json, FileType::Rust).is_ok());
        
        // Invalid JSON should fail validation
        let invalid_json = json!({
            "metadata": {},
            // Missing required fields
        });
        assert!(converter.validate_schema(&invalid_json, FileType::Rust).is_err());
    }

    #[test]
    fn test_element_conversion() {
        let converter = UnifiedJsonConverter::with_default_schemas();
        
        let element = DocumentElement::new(
            ElementType::Function,
            Some("test_func".to_string()),
            "fn test_func() {}".to_string(),
            5,
            7,
        )
        .with_attribute("visibility".to_string(), "public")
        .with_child(DocumentElement::new(
            ElementType::Variable,
            Some("x".to_string()),
            "let x = 42;".to_string(),
            6,
            6,
        ));
        
        // Need dummy doc for context
        let metadata = DocumentMetadata::new(PathBuf::from("test.rs"), 0);
        let doc = ParsedDocument::new(metadata);

        let json = converter.convert_element(&element, "", &doc);
        
        assert_eq!(json["type"], "function");
        assert_eq!(json["name"], "test_func");
        assert_eq!(json["line_start"], 5);
        assert_eq!(json["line_end"], 7);
        assert!(json["attributes"].is_object());
        // Children are now handled by group_elements_by_type, not convert_element
        assert!(json.get("children").is_none());
    }
}