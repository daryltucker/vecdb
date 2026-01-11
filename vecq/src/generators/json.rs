use crate::generator::Generator;
use crate::types::ParsedDocument;
use crate::error::VecqResult;
use serde_json::Value;

pub struct JsonGenerator;

impl Default for JsonGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonGenerator {
    pub fn new() -> Self {
        Self
    }

    fn element_to_value(element: &crate::types::DocumentElement) -> Value {
        // If it was a block, it was likely an object or array we exploded.
        // But for simplicity of "reconstruction", we can look at the "value" attribute
        // that JsonParser stores.
        if let Some(val) = element.attributes.get("value") {
            return val.clone();
        }
        
        // Fallback for elements without "value" attribute (though JsonParser adds it)
        Value::Null
    }
}

impl Generator for JsonGenerator {
    fn generate(&self, doc: &ParsedDocument) -> VecqResult<String> {
        // If we have multiple top-level elements, it might be an array or multiple keys from a root object.
        // JsonParser puts root keys/indices into doc.elements.
        
        // If we want to reconstruct the *exact* root object, we need to know if it was an object or array.
        // JsonParser handles this in its `parse` method.
        
        // This is a simplified generator for round-trip verification.
        // We'll reconstruct a Map if all elements have keys, or an Array if they look like indices.
        
        let mut map = serde_json::Map::new();
        let mut arr = Vec::new();
        let mut is_arr = true;
        
        for (i, element) in doc.elements.iter().enumerate() {
            let key = element.name.as_deref().unwrap_or("");
            if key != format!("[{}]", i) {
                is_arr = false;
            }
            
            let val = Self::element_to_value(element);
            if !is_arr {
                map.insert(key.to_string(), val.clone());
            }
            arr.push(val);
        }
        
        let final_value = if is_arr && !arr.is_empty() {
            Value::Array(arr)
        } else {
            Value::Object(map)
        };
        
        serde_json::to_string_pretty(&final_value)
            .map_err(|e| crate::error::VecqError::json_error("Failed to generate JSON".to_string(), Some(e)))
    }
}
