use crate::error::VecqResult;
use crate::generator::Generator;
use crate::types::ParsedDocument;
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

    /// Render elements as JSON text in ELEMENT order.
    ///
    /// `element_to_value` hands back a `serde_json::Value`, whose maps are
    /// alphabetical, so generating from it reordered every object. The parser
    /// emits members in document order (2026-300), so a round trip must
    /// generate from the elements themselves; only leaves use `value`.
    fn render(elements: &[crate::types::DocumentElement], indent: usize) -> String {
        let is_array = elements
            .iter()
            .enumerate()
            .all(|(i, e)| e.name.as_deref() == Some(format!("[{i}]").as_str()));
        let pad = "  ".repeat(indent + 1);
        let close = "  ".repeat(indent);
        let items: Vec<String> = elements
            .iter()
            .map(|e| {
                let value = if e.children.is_empty() {
                    let v = Self::element_to_value(e);
                    serde_json::to_string(&v).unwrap_or_else(|_| "null".to_string())
                } else {
                    Self::render(&e.children, indent + 1)
                };
                match (is_array, e.name.as_deref()) {
                    (false, Some(key)) => {
                        let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into());
                        format!("{pad}{key}: {value}")
                    }
                    _ => format!("{pad}{value}"),
                }
            })
            .collect();
        let (open, end) = if is_array { ("[", "]") } else { ("{", "}") };
        if items.is_empty() {
            return format!("{open}{end}");
        }
        format!("{open}\n{}\n{close}{end}", items.join(",\n"))
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

        // Built from the elements, in their order — see `render`. An empty
        // document is `{}`, as it was when this reconstructed a Map.
        if doc.elements.is_empty() {
            return Ok("{}".to_string());
        }
        Ok(Self::render(&doc.elements, 0))
    }
}
