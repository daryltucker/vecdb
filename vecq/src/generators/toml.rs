use crate::generator::Generator;
use crate::types::{ParsedDocument, ElementType};
use crate::error::VecqResult;
use serde_json::Value;

pub struct TomlGenerator;

impl TomlGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Generator for TomlGenerator {
    fn generate(&self, doc: &ParsedDocument) -> VecqResult<String> {
        // For TOML, we reconstruct the document from the tables (Blocks) and entries (Variables)
        // Note: This is an initial implementation that supports the current struct.
        // It heavily relies on the structure produced by TomlParser (Block=Table, Variable=KeyVal)
        
        let mut output = String::new();
        
        // 1. Process top-level entries (variables not in a block?)
        // The current parser puts everything into a "block" or "variable".
        // Top-level key-values are just variables at the root?
        // Let's check how parser parses.
        // The current parser seems to parse `toml::Value` then traverse. 
        // If it's a Table, it makes a Block. If generic value, Variable.
        
        // Strategy: 
        // We can't easily reconstruct 1:1 format (comments, whitespace) yet without a CST.
        // But for Round Trip of DATA, we can verify the semantic equivalent.
        // Wait, "Round Trip" implies `source == generate(parse(source))`.
        // If we lose formatting, that assertion fails.
        // BUT, the user said "Formatting aside".
        
        // However, standard TOML output is deterministic.
        
        // Let's iterate elements.
        for element in &doc.elements {
            match element.element_type {
                ElementType::Block => {
                    // It's a table
                    if let Some(name) = &element.name {
                        output.push_str(&format!("[{}]\n", name));
                    }
                    
                    // The children of the block are the entries
                    for child in &element.children {
                        if child.element_type == ElementType::Variable {
                            if let Some(key) = &child.name {
                                // Extract value from attributes
                                if let Some(val) = child.attributes.get("value") {
                                    output.push_str(&format!("{} = {}\n", key, json_to_toml_val(val)));
                                }
                            }
                        }
                    }
                    output.push('\n');
                },
                ElementType::Variable => {
                    // Top level key-value
                    if let Some(key) = &element.name {
                        if let Some(val) = element.attributes.get("value") {
                            output.push_str(&format!("{} = {}\n", key, json_to_toml_val(val)));
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(output.trim().to_string())
    }
}

fn json_to_toml_val(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{}\"", s), // Basic escaping?
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(), // Error?
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_to_toml_val).collect();
            format!("[{}]", items.join(", "))
        },
        Value::Object(_) => "{ ... }".to_string(), // Inline table?
    }
}
