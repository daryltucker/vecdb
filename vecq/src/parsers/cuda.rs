// PURPOSE:
//   CUDA parser implementation for vecq using tree-sitter-cpp.
//   Extends C++ parsing with CUDA-specific elements (__global__, __device__, __host__).
//   Critical for parsing GPU kernel code and device functions.
//
// RELATED FILES:
//   - src/parsers/cpp.rs - Base C++ parser reference
//   - src/types.rs - ElementType::Kernel, ElementType::DeviceFunction

use crate::error::{VecqError, VecqResult};
use crate::parser::{Parser, ParserCapabilities, ParserConfig};
use crate::types::{DocumentElement, DocumentMetadata, ElementType, FileType, ParsedDocument, CFamilyAttributes, ElementAttributes};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

/// CUDA parser that extracts CUDA-specific elements from .cu/.cuh files
#[derive(Debug, Clone)]
pub struct CudaParser {
    _config: ParserConfig,
}

impl CudaParser {
    pub fn new() -> Self {
        Self {
            _config: ParserConfig::default(),
        }
    }

    pub fn with_config(config: ParserConfig) -> Self {
        Self { _config: config }
    }

    fn extract_kernels(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut kernels = Vec::new();
        self.extract_kernels_recursive(content, tree.root_node(), &mut kernels);
        Ok(kernels)
    }

    fn extract_kernels_recursive(&self, content: &str, node: tree_sitter::Node, kernels: &mut Vec<DocumentElement>) {
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            // Look for function definitions and check if they have CUDA qualifiers
            if child.kind() == "function_definition" || child.kind() == "declaration" {
                let text = child.utf8_text(content.as_bytes()).unwrap_or("");
                
                // Check for CUDA qualifiers
                let is_global = text.contains("__global__");
                let is_device = text.contains("__device__");
                let is_host = text.contains("__host__");

                if is_global || is_device {
                    if let Some(kernel) = self.parse_cuda_function(content, child, is_global, is_device, is_host) {
                        kernels.push(kernel);
                    }
                }
            }
            
            // Recurse into namespaces
            if child.kind() == "namespace_definition" {
                self.extract_kernels_recursive(content, child, kernels);
            }
        }
    }

    fn parse_cuda_function(&self, content: &str, node: tree_sitter::Node, is_global: bool, is_device: bool, is_host: bool) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut cursor = node.walk();

        // Find the function name
        for child in node.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "identifier" {
                        name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                        break;
                    }
                }
            }
        }

        if name.is_empty() {
            return None;
        }

        let element_type = if is_global {
            ElementType::Kernel
        } else {
            ElementType::DeviceFunction
        };

        let element = DocumentElement::new(
            element_type,
            Some(name),
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        )
        .set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: {
                let mut map = HashMap::new();
                map.insert("is_global".to_string(), json!(is_global));
                map.insert("is_device".to_string(), json!(is_device));
                map.insert("is_host".to_string(), json!(is_host));
                map
            }
        }));

        Some(element)
    }

    fn extract_includes(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut includes = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "preproc_include" {
                let text = child.utf8_text(content.as_bytes()).unwrap_or("");
                let path = text.trim_start_matches("#include")
                    .trim()
                    .trim_matches(|c| c == '<' || c == '>' || c == '"');

                let is_cuda = path.contains("cuda") || path.ends_with(".cuh");

                let element = DocumentElement::new(
                    ElementType::Import,
                    Some(path.to_string()),
                    text.to_string(),
                    child.start_position().row + 1,
                    child.end_position().row + 1,
                )
                .set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
                    other: {
                        let mut map = HashMap::new();
                        map.insert("is_cuda".to_string(), json!(is_cuda));
                        map
                    }
                }));
                
                includes.push(element);
            }
        }

        Ok(includes)
    }

    fn extract_regular_functions(&self, content: &str, tree: &tree_sitter::Tree) -> VecqResult<Vec<DocumentElement>> {
        let mut functions = Vec::new();
        let root = tree.root_node();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "function_definition" {
                let text = child.utf8_text(content.as_bytes()).unwrap_or("");
                
                // Skip CUDA-qualified functions (they're extracted separately)
                if text.contains("__global__") || text.contains("__device__") {
                    continue;
                }

                if let Some(func) = self.parse_function(content, child) {
                    functions.push(func);
                }
            }
        }

        Ok(functions)
    }

    fn parse_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "identifier" {
                        name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                        break;
                    }
                }
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(DocumentElement::new(
            ElementType::Function,
            Some(name),
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        })))
    }
}

impl Default for CudaParser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Parser for CudaParser {
    fn file_extensions(&self) -> &[&str] {
        &["cu", "cuh"]
    }

    fn language_name(&self) -> &str {
        "CUDA"
    }

    fn capabilities(&self) -> ParserCapabilities {
        ParserCapabilities {
            incremental: false,
            error_recovery: true,
            documentation: false,
            type_information: true,
            macros: true,
            max_file_size: None,
        }
    }

    async fn parse(&self, content: &str) -> VecqResult<ParsedDocument> {
        // Use C++ parser for CUDA (it's a superset)
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_cpp::LANGUAGE.into())
            .map_err(|e| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: format!("Failed to set CUDA/C++ language: {}", e),
                source: None,
            })?;

        let tree = parser.parse(content, None)
            .ok_or_else(|| VecqError::ParseError {
                file: PathBuf::from("<string>"),
                line: 0,
                message: "Failed to parse CUDA content".to_string(),
                source: None,
            })?;

        let kernels = self.extract_kernels(content, &tree)?;
        let functions = self.extract_regular_functions(content, &tree)?;
        let includes = self.extract_includes(content, &tree)?;

        let mut elements = Vec::new();
        elements.extend(kernels);
        elements.extend(functions);
        elements.extend(includes);

        let mut doc = ParsedDocument::new(
            DocumentMetadata::new(PathBuf::from("file.cu"), content.len() as u64)
                .with_line_count(content)
                .with_file_type(FileType::Cuda)
        );
        doc.elements = elements;

        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_global_kernel() {
        let parser = CudaParser::new();
        let content = r#"
__global__ void vectorAdd(float* a, float* b, float* c, int n) {
    int i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i < n) c[i] = a[i] + b[i];
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let kernels: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Kernel)
            .collect();
        
        assert_eq!(kernels.len(), 1);
        assert_eq!(kernels[0].name, Some("vectorAdd".to_string()));
        assert!(kernels[0].attributes.get("is_global").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_parse_device_function() {
        let parser = CudaParser::new();
        let content = r#"
__device__ float square(float x) {
    return x * x;
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let device_funcs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::DeviceFunction)
            .collect();
        
        assert_eq!(device_funcs.len(), 1);
        assert_eq!(device_funcs[0].name, Some("square".to_string()));
        assert!(device_funcs[0].attributes.get("is_device").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_parse_host_device() {
        let parser = CudaParser::new();
        let content = r#"
__host__ __device__ float add(float a, float b) {
    return a + b;
}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let funcs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::DeviceFunction)
            .collect();
        
        assert_eq!(funcs.len(), 1);
        assert!(funcs[0].attributes.get("is_host").unwrap().as_bool().unwrap());
        assert!(funcs[0].attributes.get("is_device").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_parse_cuda_includes() {
        let parser = CudaParser::new();
        let content = r#"
#include <cuda_runtime.h>
#include "mykernel.cuh"
#include <stdio.h>

int main() { return 0; }
"#;
        let result = parser.parse(content).await.unwrap();
        
        let imports: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Import)
            .collect();
        
        assert_eq!(imports.len(), 3);
        // cuda_runtime.h and mykernel.cuh should be marked as CUDA
        assert!(imports[0].attributes.get("is_cuda").unwrap().as_bool().unwrap());
        assert!(imports[1].attributes.get("is_cuda").unwrap().as_bool().unwrap());
        assert!(!imports[2].attributes.get("is_cuda").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_parse_mixed_functions() {
        let parser = CudaParser::new();
        let content = r#"
__global__ void kernel() {}
void hostFunc() {}
__device__ void deviceFunc() {}
"#;
        let result = parser.parse(content).await.unwrap();
        
        let kernels: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Kernel)
            .collect();
        let device_funcs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::DeviceFunction)
            .collect();
        let host_funcs: Vec<_> = result.elements.iter()
            .filter(|e| e.element_type == ElementType::Function)
            .collect();
        
        assert_eq!(kernels.len(), 1);
        assert_eq!(device_funcs.len(), 1);
        assert_eq!(host_funcs.len(), 1);
    }

    #[tokio::test]
    async fn test_empty_content() {
        let parser = CudaParser::new();
        let result = parser.parse("").await.unwrap();
        assert!(result.elements.is_empty());
    }
}
