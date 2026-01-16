use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Represents a single versioned snapshot of an artifact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Text content of the snapshot
    pub content: String,
    /// Associated metadata (from vecq or file system)
    pub metadata: Value,
}

impl Snapshot {
    pub fn new(content: String, metadata: Value) -> Self {
        Self { content, metadata }
    }
}

/// Reason for a timeline branch
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "details")]
pub enum BranchReason {
    /// Root timeline (no parent)
    Root,
    /// Detected massive rewrite
    MassiveRewrite {
        deletion_ratio: f32,
        overlap_ratio: f32,
    },
    /// Detected semantic shift (e.g. header changes)
    SemanticShift, // Placeholder for Phase 2b
    /// Explicit creation
    Manual,
}

/// Represents a distinct narrative timeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    /// The artifact this timeline belongs to
    pub artifact: String,
    /// Unique identifier (e.g. "main", "rewrite_v20")
    pub id: String,
    /// Parent timeline ID (if branched)
    pub parent_id: Option<String>,
    /// Version in parent where this branched
    pub branch_point: Option<usize>,
    /// Why this timeline was created
    pub reason: BranchReason,
}

/// An event in the evolution of an artifact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionEvent {
    pub event_type: String, // "creation", "evolution"
    pub artifact: String,
    pub timeline_id: String,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<usize>, // For creation
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_from: Option<usize>, // For evolution
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_to: Option<usize>, // For evolution
    
    pub diff_summary: String,
    pub full_content: String,
    pub timestamp: Option<Value>,
}

/// Container for the full timeline analysis output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineAnalysis {
    pub timelines: Vec<Timeline>,
    pub events: Vec<EvolutionEvent>,
}
