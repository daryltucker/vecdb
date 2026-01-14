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

    fn process_nodes(
        &self,
        node: tree_sitter::Node,
        content: &str,
        source: &[u8],
    ) -> Vec<DocumentElement> {
        let mut elements = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            let kind = child.kind();
            
            match kind {
                "namespace_definition" => {
                   if let Some(ns) = self.parse_namespace(content, child) {
                       let mut children = Vec::new();
                       if let Some(body) = child.child_by_field_name("body") {
                           children = self.process_nodes(body, content, source);
                       }
                       elements.push(ns.with_children(children));
                   }
                }
                "function_definition" => {
                    if let Some(func) = self.parse_cuda_function(content, child) {
                        elements.push(func);
                    }
                }
                "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
                    let element_type = match kind {
                         "class_specifier" => ElementType::Class,
                         "struct_specifier" => ElementType::Struct,
                         "union_specifier" => ElementType::Union,
                         "enum_specifier" => ElementType::Enum,
                         _ => ElementType::Struct,
                    };
                    
                    let effective_type = match element_type {
                         ElementType::Class | ElementType::Struct | ElementType::Union | ElementType::Enum => element_type,
                         _ => ElementType::Struct,
                    };

                    let mut element = self.parse_complex_type(content, child, effective_type);
                    
                    if let Some(body) = child.child_by_field_name("body") {
                        let children = self.process_nodes(body, content, source);
                        element = element.with_children(children);
                    }
                    elements.push(element);
                }
                 "preproc_include" => {
                     if let Some(include) = self.parse_include(content, child) {
                         elements.push(include);
                     }
                }
                "declaration" => {
                     // Fields, Variables, or Function Declarations (prototypes)
                     // If we are in a struct/class, these are fields.
                     // If we are global, these are variables or prototypes.
                     // For hierarchy, we care about fields inside structs.
                     if let Some(field) = self.parse_field(content, child) {
                         elements.push(field);
                     }
                }
                 "template_declaration" => {
                    // Handle templates by processing the declaration inside
                     // Just recurse immediate children
                     // A template_declaration usually has a child like function_definition or class_specifier
                     // We can process those.
                      // Elements inside template might duplicate if we just call process_nodes on children?
                      // No, template_declaration wraps them.
                      // We should likely extract attributes from template_declaration (template params) and attach to child.
                      // For now, let's just descend.
                      // Actually, if we descend, we might lose the "Template" context, but we get the function/class.
                      // That is acceptable for now.
                      // We can assume the child will be picked up.
                      // Wait, if I call process_nodes(child, ...), it iterates children of child.
                      // template_declaration structure: `template <...> declaration`.
                      // The declaration child (e.g. function_definition) is what we want.
                      // But `process_nodes` iterates children of the node passed.
                      // So if I pass `child` (template_decl), `process_nodes` iterates its children.
                      // One child is `function_definition`. It will match "function_definition" case.
                      // So elements.extend(self.process_nodes(child, ...)) works.
                      elements.extend(self.process_nodes(child, content, source));
                }
                _ => {
                    if kind == "linkage_specification" {
                        if let Some(body) = child.child_by_field_name("body") {
                             elements.extend(self.process_nodes(body, content, source));
                        }
                    }
                }
            }
        }
        
        elements
    }

    fn parse_namespace(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
         let mut name = None;
         let mut cursor = node.walk();
         
         for child in node.children(&mut cursor) {
             if child.kind() == "identifier" || child.kind() == "namespace_identifier" {
                 name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
             }
         }
         
         Some(DocumentElement::new(
             ElementType::Namespace,
             name,
             node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
             node.start_position().row + 1,
             node.end_position().row + 1,
         ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
             other: HashMap::new(),
         })))
    }

    fn parse_cuda_function(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let mut name = String::new();
        
        // Find name
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_declarator" {
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "identifier" || decl_child.kind() == "field_identifier" {
                        name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                    } else if decl_child.kind() == "qualified_identifier" {
                         name = decl_child.utf8_text(content.as_bytes()).unwrap_or("").to_string();
                    }
                }
            }
        }
        
        if name.is_empty() {
             // Fallback for some definitions
             if let Some(declarator) = node.child_by_field_name("declarator") {
                 let text = declarator.utf8_text(content.as_bytes()).unwrap_or("");
                 // rough extraction if complex declarator
                 name = text.split('(').next().unwrap_or("").trim().to_string(); 
                 // This might include * or &
                 if let Some(idx) = name.rfind(|c: char| !c.is_alphanumeric() && c != '_') {
                     name = name[idx+1..].to_string();
                 }
             }
        }
        
        if name.is_empty() {
            return None;
        }

        let full_text = node.utf8_text(content.as_bytes()).unwrap_or("");
        let is_global = full_text.contains("__global__");
        let is_device = full_text.contains("__device__");
        let is_host = full_text.contains("__host__"); // explicit host or implicit?
        // Implicit host if neither global or device? No, __host__ can be explicit.
        // If neither, it is standard Function.
        
        let element_type = if is_global {
            ElementType::Kernel
        } else if is_device {
            ElementType::DeviceFunction
        } else {
            ElementType::Function
        };

        Some(DocumentElement::new(
            element_type,
            Some(name),
            full_text.to_string(),
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
        })))
    }
    
    fn parse_complex_type(&self, content: &str, node: tree_sitter::Node, element_type: ElementType) -> DocumentElement {
        let mut name: Option<String> = None;
        let mut cursor = node.walk();
        
        for child in node.children(&mut cursor) {
             if child.kind() == "type_identifier" || child.kind() == "identifier" {
                  name = Some(child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                  break;
             }
        }

        DocumentElement::new(
            element_type,
            name,
            node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: HashMap::new(),
        }))
    }
    
    fn parse_include(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
        let text = node.utf8_text(content.as_bytes()).unwrap_or("");
        let path = text.trim_start_matches("#include")
            .trim()
            .trim_matches(|c| c == '<' || c == '>' || c == '"');
        let is_cuda = path.contains("cuda") || path.ends_with(".cuh") || path.ends_with(".cu");

        Some(DocumentElement::new(
            ElementType::Import,
            Some(path.to_string()),
            text.to_string(),
            node.start_position().row + 1,
            node.end_position().row + 1,
        ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
            other: {
                let mut map = HashMap::new();
                map.insert("is_cuda".to_string(), json!(is_cuda));
                map
            }
        })))
    }
    
    fn parse_field(&self, content: &str, node: tree_sitter::Node) -> Option<DocumentElement> {
         let mut name = None;
         let mut cursor = node.walk();
        
        for child in node.children(&mut cursor) {
             if child.kind() == "init_declarator" || child.kind() == "field_declaration" {
                  let mut d_cursor = child.walk();
                   for d_child in child.children(&mut d_cursor) {
                        if d_child.kind() == "identifier" || d_child.kind() == "field_identifier" {
                            name = Some(d_child.utf8_text(content.as_bytes()).unwrap_or("").to_string());
                            break;
                        }
                   }
             }
        }
        
        if let Some(n) = name {
            Some(DocumentElement::new(
                ElementType::Variable,
                Some(n),
                node.utf8_text(content.as_bytes()).unwrap_or("").to_string(),
                node.start_position().row + 1,
                node.end_position().row + 1,
            ).set_attributes(ElementAttributes::CFamily(CFamilyAttributes {
                other: HashMap::new(),
            })))
        } else {
            None
        }
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

        let elements = self.process_nodes(tree.root_node(), content, content.as_bytes());

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
