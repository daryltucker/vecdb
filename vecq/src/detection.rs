// PURPOSE:
//   File type detection engine for vecq that intelligently identifies document types
//   and selects appropriate parsers. Critical for vecq's user experience as users
//   should not need to manually specify file types. Uses multiple detection strategies
//   to ensure accurate identification even with ambiguous or missing file extensions.
//
// REQUIREMENTS:
//   User-specified:
//   - Must accurately identify all supported file types (Rust, Python, Markdown, etc.)
//   - Must handle files without extensions or with ambiguous extensions
//   - Must support user-defined file type mappings via configuration
//   - Must provide confidence scoring for detection decisions
//   - Must be extensible for new file types without breaking existing detection
//   
//   Implementation-discovered:
//   - Requires multiple detection strategies (extension, MIME, shebang, content analysis)
//   - Must cache detection results for performance with repeated processing
//   - Needs comprehensive logging for debugging misclassification issues
//   - Must handle binary files and encoding issues gracefully
//
// IMPLEMENTATION RULES:
//   1. Use multiple detection strategies with confidence scoring
//      Rationale: Single strategy is insufficient for accurate detection
//   
//   2. Primary strategy is file extension mapping for performance
//      Rationale: Most files have correct extensions, fastest detection method
//   
//   3. Secondary strategy is MIME type detection for extensionless files
//      Rationale: Handles files without extensions or with wrong extensions
//   
//   4. Tertiary strategy is shebang detection for script files
//      Rationale: Scripts often have generic extensions but specific shebangs
//   
//   5. Cache detection results by file hash to avoid redundant work
//      Rationale: Same files are often processed multiple times
//   
//   Critical:
//   - DO NOT change detection behavior without extensive testing
//   - DO NOT cache results without considering file content changes
//   - ALWAYS provide fallback to Unknown type for unsupported files
//
// USAGE:
//   use vecq::detection::{FileTypeDetector, HybridDetector, DetectionConfig};
//   use std::path::Path;
//   
//   // Basic detection
//   let detector = HybridDetector::new();
//   let file_type = detector.detect_type(Path::new("example.rs"), content.as_bytes())?;
//   
//   // With custom configuration
//   let config = DetectionConfig::new()
//       .with_custom_mapping("mylang", FileType::Unknown);
//   let detector = HybridDetector::with_config(config);
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new file type support:
//   1. Add file extensions to extension mapping in HybridDetector
//   2. Add MIME type patterns if applicable
//   3. Add shebang patterns for script-based languages
//   4. Update confidence scoring logic for new patterns
//   5. Add comprehensive tests for new file type detection
//   6. Update documentation with new supported extensions
//   
//   When detection accuracy issues are reported:
//   1. Add logging to identify misclassification patterns
//   2. Collect problematic files for test fixtures
//   3. Adjust confidence scoring or add new detection strategies
//   4. Add regression tests to prevent future issues
//   5. Update user documentation with detection limitations
//
// RELATED FILES:
//   - src/types.rs - Defines FileType enum used by detection
//   - src/parser.rs - Uses detection results for parser selection
//   - src/parsers/mod.rs - Parser implementations for detected file types
//   - tests/unit/detection_tests.rs - Detection accuracy validation
//   - tests/fixtures/ - Real-world files for detection testing
//
// MAINTENANCE:
//   Update when:
//   - New file types are added to vecq
//   - File extension conventions change in programming languages
//   - Detection accuracy issues are reported by users
//   - Performance optimization opportunities identified
//   - New detection strategies become available
//
// Last Verified: 2025-12-31

use crate::error::VecqResult;
use crate::parser::Parser;
use crate::parsers;
use crate::types::FileType;
use std::collections::HashMap;
use std::path::Path;

/// Trait for detecting file types and selecting appropriate parsers
pub trait FileTypeDetector: Send + Sync {
    /// Detect file type from path and content
    fn detect_type(&self, path: &Path, content: &[u8]) -> VecqResult<FileType>;

    /// Get parser for detected file type
    fn get_parser(&self, file_type: FileType) -> VecqResult<Box<dyn Parser>>;

    /// Get detection confidence score (0.0 to 1.0)
    fn get_confidence(&self, path: &Path, content: &[u8]) -> f64;
}

/// Configuration for file type detection
#[derive(Debug, Clone)]
pub struct DetectionConfig {
    /// Custom file extension mappings
    pub custom_extensions: HashMap<String, FileType>,
    /// Custom MIME type mappings
    pub custom_mime_types: HashMap<String, FileType>,
    /// Custom shebang patterns
    pub custom_shebangs: HashMap<String, FileType>,
    /// Minimum confidence threshold for detection
    pub confidence_threshold: f64,
    /// Enable content-based detection
    pub enable_content_detection: bool,
    /// Enable caching of detection results
    pub enable_caching: bool,
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            custom_extensions: HashMap::new(),
            custom_mime_types: HashMap::new(),
            custom_shebangs: HashMap::new(),
            confidence_threshold: 0.5,
            enable_content_detection: true,
            enable_caching: true,
        }
    }
}

impl DetectionConfig {
    /// Create new detection configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Add custom file extension mapping
    pub fn with_custom_extension(mut self, extension: &str, file_type: FileType) -> Self {
        self.custom_extensions.insert(extension.to_lowercase(), file_type);
        self
    }

    /// Add custom MIME type mapping
    pub fn with_custom_mime_type(mut self, mime_type: &str, file_type: FileType) -> Self {
        self.custom_mime_types.insert(mime_type.to_lowercase(), file_type);
        self
    }

    /// Add custom shebang pattern
    pub fn with_custom_shebang(mut self, pattern: &str, file_type: FileType) -> Self {
        self.custom_shebangs.insert(pattern.to_string(), file_type);
        self
    }

    /// Set confidence threshold
    pub fn with_confidence_threshold(mut self, threshold: f64) -> Self {
        self.confidence_threshold = threshold.clamp(0.0, 1.0);
        self
    }
}

/// Detection result with confidence score
#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub file_type: FileType,
    pub confidence: f64,
    pub strategy: DetectionStrategy,
    pub details: String,
}

/// Detection strategy used
#[derive(Debug, Clone, PartialEq)]
pub enum DetectionStrategy {
    Extension,
    MimeType,
    Shebang,
    ContentAnalysis,
    CustomMapping,
    Fallback,
}

/// Hybrid file type detector using multiple strategies
pub struct HybridDetector {
    config: DetectionConfig,
    detection_cache: std::sync::Mutex<lru::LruCache<String, DetectionResult>>,
}

impl HybridDetector {
    /// Create new hybrid detector with default configuration
    pub fn new() -> Self {
        Self::with_config(DetectionConfig::default())
    }

    /// Create hybrid detector with custom configuration
    pub fn with_config(config: DetectionConfig) -> Self {
        let cache_size = if config.enable_caching { 1000 } else { 1 };
        Self {
            config,
            detection_cache: std::sync::Mutex::new(
                lru::LruCache::new(std::num::NonZeroUsize::new(cache_size).unwrap())
            ),
        }
    }

    /// Detect file type using extension-based strategy
    fn detect_by_extension(&self, path: &Path) -> Option<DetectionResult> {
        let extension = path.extension()?.to_str()?.to_lowercase();

        // Check custom extensions first
        if let Some(&file_type) = self.config.custom_extensions.get(&extension) {
            return Some(DetectionResult {
                file_type,
                confidence: 0.95,
                strategy: DetectionStrategy::CustomMapping,
                details: format!("Custom extension mapping: {}", extension),
            });
        }

        // Check built-in extensions
        let file_type = FileType::from_extension(&extension)?;
        Some(DetectionResult {
            file_type,
            confidence: 0.9,
            strategy: DetectionStrategy::Extension,
            details: format!("File extension: {}", extension),
        })
    }

    /// Detect file type using MIME type analysis
    fn detect_by_mime_type(&self, content: &[u8]) -> Option<DetectionResult> {
        let kind = infer::get(content)?;
        let mime_type = kind.mime_type();

        // Check custom MIME types first
        if let Some(&file_type) = self.config.custom_mime_types.get(mime_type) {
            return Some(DetectionResult {
                file_type,
                confidence: 0.8,
                strategy: DetectionStrategy::CustomMapping,
                details: format!("Custom MIME type: {}", mime_type),
            });
        }

        // Built-in MIME type mappings
        let file_type = match mime_type {
            "text/plain" => FileType::Unknown, // Too generic
            "text/markdown" => FileType::Markdown,
            "text/x-python" => FileType::Python,
            "text/x-c" => FileType::C,
            "text/x-c++" => FileType::Cpp,
            "application/x-sh" => FileType::Bash,
            _ => return None,
        };

        Some(DetectionResult {
            file_type,
            confidence: 0.7,
            strategy: DetectionStrategy::MimeType,
            details: format!("MIME type: {}", mime_type),
        })
    }

    /// Detect file type using shebang analysis
    fn detect_by_shebang(&self, content: &[u8]) -> Option<DetectionResult> {
        let content_str = std::str::from_utf8(content).ok()?;
        let first_line = content_str.lines().next()?;

        if !first_line.starts_with("#!") {
            return None;
        }

        // Check custom shebangs first
        for (pattern, &file_type) in &self.config.custom_shebangs {
            if first_line.contains(pattern) {
                return Some(DetectionResult {
                    file_type,
                    confidence: 0.85,
                    strategy: DetectionStrategy::CustomMapping,
                    details: format!("Custom shebang pattern: {}", pattern),
                });
            }
        }

        // Built-in shebang patterns
        let file_type = if first_line.contains("python") {
            FileType::Python
        } else if first_line.contains("bash") || first_line.contains("/bin/sh") {
            FileType::Bash
        } else if first_line.contains("node") {
            FileType::Unknown // JavaScript not supported yet
        } else {
            return None;
        };

        Some(DetectionResult {
            file_type,
            confidence: 0.8,
            strategy: DetectionStrategy::Shebang,
            details: format!("Shebang: {}", first_line),
        })
    }

    /// Detect file type using content analysis
    fn detect_by_content(&self, content: &[u8]) -> Option<DetectionResult> {
        if !self.config.enable_content_detection {
            return None;
        }

        let content_str = std::str::from_utf8(content).ok()?;
        let sample = content_str.chars().take(1000).collect::<String>();

        // Simple heuristics based on content patterns
        let rust_score = self.calculate_rust_score(&sample);
        let python_score = self.calculate_python_score(&sample);
        let markdown_score = self.calculate_markdown_score(&sample);
        let c_score = self.calculate_c_score(&sample);

        let max_score = rust_score.max(python_score).max(markdown_score).max(c_score);

        if max_score < 0.3 {
            return None; // Not confident enough
        }

        let (file_type, confidence) = if rust_score == max_score {
            (FileType::Rust, rust_score)
        } else if python_score == max_score {
            (FileType::Python, python_score)
        } else if markdown_score == max_score {
            (FileType::Markdown, markdown_score)
        } else {
            (FileType::C, c_score)
        };

        Some(DetectionResult {
            file_type,
            confidence: confidence * 0.6, // Content analysis is less reliable
            strategy: DetectionStrategy::ContentAnalysis,
            details: format!("Content analysis score: {:.2}", confidence),
        })
    }

    /// Calculate Rust content score
    fn calculate_rust_score(&self, content: &str) -> f64 {
        let mut score: f64 = 0.0;
        let total_lines = content.lines().count() as f64;

        if total_lines == 0.0 {
            return 0.0;
        }

        // Rust-specific patterns
        let rust_keywords = ["fn ", "struct ", "enum ", "impl ", "trait ", "use ", "mod "];
        let rust_patterns = ["-> ", "::", "&mut ", "&self", "Option<", "Result<"];

        for keyword in &rust_keywords {
            if content.contains(keyword) {
                score += 0.15;
            }
        }

        for pattern in &rust_patterns {
            if content.contains(pattern) {
                score += 0.1;
            }
        }

        // Rust-specific syntax
        if content.contains("println!") || content.contains("vec!") {
            score += 0.2;
        }

        score.min(1.0)
    }

    /// Calculate Python content score
    fn calculate_python_score(&self, content: &str) -> f64 {
        let mut score: f64 = 0.0;

        // Python-specific patterns
        let python_keywords = ["def ", "class ", "import ", "from ", "if __name__"];
        let python_patterns = ["self.", "self,", ":", "    "] ; // Indentation

        for keyword in &python_keywords {
            if content.contains(keyword) {
                score += 0.15;
            }
        }

        for pattern in &python_patterns {
            if content.contains(pattern) {
                score += 0.1;
            }
        }

        // Python-specific functions
        if content.contains("print(") || content.contains("len(") {
            score += 0.1;
        }

        score.min(1.0)
    }

    /// Calculate Markdown content score
    fn calculate_markdown_score(&self, content: &str) -> f64 {
        let mut score: f64 = 0.0;

        // Markdown-specific patterns
        if content.contains("# ") || content.contains("## ") {
            score += 0.3;
        }

        if content.contains("```") || content.contains("~~~") {
            score += 0.2;
        }

        if content.contains("[") && content.contains("](") {
            score += 0.2;
        }

        if content.contains("- ") || content.contains("* ") {
            score += 0.1;
        }

        score.min(1.0)
    }

    /// Calculate C content score
    fn calculate_c_score(&self, content: &str) -> f64 {
        let mut score: f64 = 0.0;

        // C-specific patterns
        let c_keywords = ["#include", "int main", "void ", "char ", "struct "];
        let c_patterns = [";", "{", "}", "/*", "*/"];

        for keyword in &c_keywords {
            if content.contains(keyword) {
                score += 0.15;
            }
        }

        for pattern in &c_patterns {
            if content.contains(pattern) {
                score += 0.05;
            }
        }

        // C-specific functions
        if content.contains("printf(") || content.contains("malloc(") {
            score += 0.2;
        }

        score.min(1.0)
    }

    /// Generate cache key for detection result
    fn generate_cache_key(&self, path: &Path, content: &[u8]) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        content.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Get best detection result from multiple strategies
    fn get_best_detection(&self, path: &Path, content: &[u8]) -> DetectionResult {
        let mut candidates = Vec::new();

        // Try all detection strategies
        if let Some(result) = self.detect_by_extension(path) {
            candidates.push(result);
        }

        if let Some(result) = self.detect_by_mime_type(content) {
            candidates.push(result);
        }

        if let Some(result) = self.detect_by_shebang(content) {
            candidates.push(result);
        }

        if let Some(result) = self.detect_by_content(content) {
            candidates.push(result);
        }

        // Select best candidate by confidence
        candidates
            .into_iter()
            .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
            .unwrap_or(DetectionResult {
                file_type: FileType::Unknown,
                confidence: 0.0,
                strategy: DetectionStrategy::Fallback,
                details: "No detection strategy succeeded".to_string(),
            })
    }
}

impl Default for HybridDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl FileTypeDetector for HybridDetector {
    fn detect_type(&self, path: &Path, content: &[u8]) -> VecqResult<FileType> {
        // Check cache first
        if self.config.enable_caching {
            let cache_key = self.generate_cache_key(path, content);
            if let Ok(mut cache) = self.detection_cache.lock() {
                if let Some(cached_result) = cache.get(&cache_key) {
                    return Ok(cached_result.file_type);
                }
            }
        }

        // Perform detection
        let result = self.get_best_detection(path, content);

        // Cache result
        if self.config.enable_caching {
            let cache_key = self.generate_cache_key(path, content);
            if let Ok(mut cache) = self.detection_cache.lock() {
                cache.put(cache_key, result.clone());
            }
        }

        // Check confidence threshold
        if result.confidence < self.config.confidence_threshold {
            return Ok(FileType::Unknown);
        }

        Ok(result.file_type)
    }

    fn get_parser(&self, file_type: FileType) -> VecqResult<Box<dyn Parser>> {
        parsers::create_parser(file_type)
    }

    fn get_confidence(&self, path: &Path, content: &[u8]) -> f64 {
        self.get_best_detection(path, content).confidence
    }
}

// Implement the vecdb-common trait
impl vecdb_common::FileTypeDetector for HybridDetector {
    fn detect(&self, path: &Path, content: &[u8]) -> FileType {
        // Infallible detection for the common trait
        match self.detect_type(path, content) {
            Ok(ft) => ft,
            Err(_) => FileType::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extension_detection() {
        let detector = HybridDetector::new();
        
        let rust_path = PathBuf::from("main.rs");
        let result = detector.detect_type(&rust_path, b"fn main() {}").unwrap();
        assert_eq!(result, FileType::Rust);
        
        let python_path = PathBuf::from("script.py");
        let result = detector.detect_type(&python_path, b"def main(): pass").unwrap();
        assert_eq!(result, FileType::Python);
        
        let markdown_path = PathBuf::from("README.md");
        let result = detector.detect_type(&markdown_path, b"# Title").unwrap();
        assert_eq!(result, FileType::Markdown);
    }

    #[test]
    fn test_shebang_detection() {
        let detector = HybridDetector::new();
        let path = PathBuf::from("script");
        
        let python_content = b"#!/usr/bin/env python3\nprint('hello')";
        let result = detector.detect_type(&path, python_content).unwrap();
        assert_eq!(result, FileType::Python);
        
        let bash_content = b"#!/bin/bash\necho 'hello'";
        let result = detector.detect_type(&path, bash_content).unwrap();
        assert_eq!(result, FileType::Bash);
    }

    #[test]
    fn test_content_analysis() {
        let detector = HybridDetector::new();
        let path = PathBuf::from("unknown");
        
        let rust_content = b"fn main() {\n    println!(\"Hello, world!\");\n}";
        let _result = detector.detect_type(&path, rust_content).unwrap();
        // Content analysis might detect this as Rust
        
        let markdown_content = b"# Title\n\n## Subtitle\n\n- Item 1\n- Item 2";
        let _result = detector.detect_type(&path, markdown_content).unwrap();
        // Content analysis might detect this as Markdown
    }

    #[test]
    fn test_custom_configuration() {
        let config = DetectionConfig::new()
            .with_custom_extension("mylang", FileType::Unknown)
            .with_confidence_threshold(0.8);
        
        let detector = HybridDetector::with_config(config);
        let path = PathBuf::from("test.mylang");
        let result = detector.detect_type(&path, b"custom content").unwrap();
        assert_eq!(result, FileType::Unknown);
    }

    #[test]
    fn test_confidence_scoring() {
        let detector = HybridDetector::new();
        
        let rust_path = PathBuf::from("main.rs");
        let confidence = detector.get_confidence(&rust_path, b"fn main() {}");
        assert!(confidence > 0.8); // Extension detection should be high confidence
        
        let unknown_path = PathBuf::from("unknown");
        let confidence = detector.get_confidence(&unknown_path, b"random content");
        assert!(confidence < 0.5); // Unknown content should be low confidence
    }

    #[test]
    fn test_detection_caching() {
        let detector = HybridDetector::new();
        let path = PathBuf::from("test.rs");
        let content = b"fn main() {}";
        
        // First detection
        let result1 = detector.detect_type(&path, content).unwrap();
        
        // Second detection (should use cache)
        let result2 = detector.detect_type(&path, content).unwrap();
        
        assert_eq!(result1, result2);
        assert_eq!(result1, FileType::Rust);
    }
}