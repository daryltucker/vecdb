use crate::generator::Generator;
use crate::types::ParsedDocument;
use crate::error::VecqResult;

pub struct RustGenerator;

impl RustGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Generator for RustGenerator {
    fn generate(&self, doc: &ParsedDocument) -> VecqResult<String> {
        // RustParser stores the full source of implementation items in `content`.
        // So for high-fidelity round-tripping of logical units, we can just concatenate them.
        // NOTE: This assumes the parser captures *everything* as an element.
        // Items like comments outside elements might be lost if not captured in a "Block" or similar.
        
        let mut output = String::new();
        let mut first = true;

        for element in &doc.elements {
             if !first {
                output.push_str("\n\n");
            }
            // Just append the content. 
            // In the future, if we break down elements further, we might need to reconstruct.
            output.push_str(&element.content);
            first = false;
        }
        
        Ok(output)
    }
}
