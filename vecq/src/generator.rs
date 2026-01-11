use crate::types::ParsedDocument;
use crate::error::VecqResult;

/// A trait for generating source code from a parsed document
pub trait Generator {
    /// Generate source code from a parsed document
    fn generate(&self, doc: &ParsedDocument) -> VecqResult<String>;
}
