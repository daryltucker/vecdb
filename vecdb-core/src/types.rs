/*
 * PURPOSE:
 *   Defines the core data structures used throughout the vecdb-mcp system.
 *   These types act as the common language between the CLI, MCP Server,
 *   and Storage Backends.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - Must support "Document Ingestion" (R-001)
 *   - Must support "Embedding Generation" (R-004)
 *   - Must preserve metadata (R-007)
 *
 *   Implementation-discovered:
 *   - Needs Serialization (Serde) for MCP/JSON transport
 *   - Needs Clone/Debug for developer ergonomics
 *   - Split Document vs Chunk to represent Source vs Vectorized units
 *
 * IMPLEMENTATION RULES:
 *   1. Use `HashMap<String, serde_json::Value>` for metadata
 *      Rationale: Maximum flexibility for arbitrary user data (Law #1 Config is Code)
 *
 *   2. Vectors are `Vec<f32>`
 *      Rationale: Standard representation for ONNX/Qdrant
 *
 *   3. IDs are UUIDs
 *      Rationale: Collision-free distributed generation suitable for large datasets
 *
 * USAGE:
 *   use vecdb_core::types::{Document, Chunk};
 *   let doc = Document::new("path/to/file.txt", "content");
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If new metadata fields required: Update struct and add migration logic/Option types
 *   - If vector dimensions change: `vectors` field is generic `Vec<f32>`, so logic is runtime-dependent
 *
 * RELATED FILES:
 *   - docs/INGESTION_DESIGN.md - Defines the chunking strategy
 *   - src/backend.rs - Consumes these types
 *
 * MAINTENANCE:
 *   Update when:
 *   - Qdrant/Backend API changes require new fields
 *   - MCP Protocol adds new resource primitives
 */

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ARCHITECTURE NOTE:
// This struct uses HashMap<String, Value> for metadata.
// In high-scale environments (>1M vectors), this causes significant heap fragmentation and overhead.
// Future Refactor (Sprint 2026-02): Replace with `bilge` bit-packed structs or `rkyv` zero-copy maps.
// See: docs/inquiries/responses/RustMemoryFilesandArchitecture.md
/// Represents a source file or logical document before ingestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Document {
    pub id: String,
    pub path: String,
    pub content: String,
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Document {
    pub fn new(path: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            path: path.into(),
            content: content.into(),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now(),
        }
    }

    pub fn with_metadata(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }
}

/// A semantic unit of a document (paragraph, sentence) with an associated vector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    pub document_id: String,
    pub content: String,
    pub vector: Option<Vec<f32>>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub page_num: Option<usize>,
    pub char_start: usize,
    pub char_end: usize,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

impl Chunk {
    pub fn new(document_id: &str, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            document_id: document_id.to_string(),
            content: content.into(),
            vector: None,
            metadata: HashMap::new(),
            page_num: None,
            char_start: 0,
            char_end: 0,
            start_line: None,
            end_line: None,
        }
    }
}

/// Result from a semantic search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub content: String,
    pub document_id: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Information about a vector collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub name: String,
    pub vector_count: Option<u64>,
    pub vector_size: Option<u64>,
    pub quantization: Option<crate::config::QuantizationType>,
}

/// Information about a background task in the backend (e.g., optimization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub collection: Option<String>,
    pub status: String, // "running", "completed", "failed"
    pub progress: Option<f32>,
    pub description: String,
}

/// Status of a local ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed(String),
}

/// Represents a long-running local ingestion job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub job_type: String, // "ingest", "history"
    pub collection: String,
    pub status: JobStatus,
    pub progress: f32, // 0.0 to 1.0
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
