use crate::types::{BranchReason, EvolutionEvent, Snapshot, Timeline, TimelineAnalysis};
use anyhow::{bail, Result};
use regex::Regex;
use serde_json::{to_value, Value};
use similar::{ChangeTag, TextDiff};
use std::collections::{BTreeMap, HashMap};

/// Trait to abstract file reading for State Strategy.
/// Allows mocking for Tier 2 tests without touching the filesystem.
pub trait SnapshotLoader {
    fn load_content(&self, path: &str) -> Option<String>;
}

/// Real implementation that reads from the filesystem
pub struct FileSystemSnapshotLoader;

impl SnapshotLoader for FileSystemSnapshotLoader {
    fn load_content(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

// Heuristics for "Massive Rewrite" detection
const MASSIVE_DELETION_THRESHOLD: f32 = 0.5;
const OVERLAP_THRESHOLD: f32 = 0.3;

/// Detects if the transition from `prev` to `curr` represents a divergent timeline branch.
fn detect_split(prev: &Snapshot, curr: &Snapshot) -> Option<BranchReason> {
    let diff = TextDiff::from_lines(&prev.content, &curr.content);

    let mut deletions = 0;
    let mut shared = 0;
    let mut total_prev_lines = 0;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                deletions += 1;
                total_prev_lines += 1;
            }
            ChangeTag::Equal => {
                shared += 1;
                total_prev_lines += 1;
            }
            ChangeTag::Insert => {}
        }
    }

    // Safety check to avoid division by zero on empty files
    if total_prev_lines == 0 {
        return None;
    }

    let del_ratio = deletions as f32 / total_prev_lines as f32;
    let overlap_ratio = shared as f32 / total_prev_lines as f32;

    if del_ratio > MASSIVE_DELETION_THRESHOLD && overlap_ratio < OVERLAP_THRESHOLD {
        return Some(BranchReason::MassiveRewrite {
            deletion_ratio: del_ratio,
            overlap_ratio: overlap_ratio,
        });
    }

    None
}

/// Processes the input JSON value using the State Reduction strategy.
///
/// 1. Expects a JSON Array of records (from `vecq --slurp`).
/// 2. Groups records by artifact base name.
/// 3. Reads full content using the `loader`.
/// 4. Generates evolution events and detects timeline splits if enabled.
pub fn process_state(
    input: Value,
    loader: &impl SnapshotLoader,
    detect_timelines: bool,
) -> Result<Value> {
    // 1. Validate Input
    let records = match input {
        Value::Array(arr) => arr,
        _ => bail!("State strategy requires a JSON array as input"),
    };

    if records.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }

    // 2. Group by Artifact
    let mut artifacts: HashMap<String, BTreeMap<usize, Snapshot>> = HashMap::new();
    let re = Regex::new(r"(.+?)\.resolved\.(\d+)$").unwrap();

    for record in records {
        let path_str = record
            .get("metadata")
            .and_then(|m| m.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if let Some(caps) = re.captures(path_str) {
            let base_path = caps.get(1).unwrap().as_str().to_string();
            let version_str = caps.get(2).unwrap().as_str();

            if let Ok(version) = version_str.parse::<usize>() {
                let content = loader.load_content(path_str).unwrap_or_default();
                let snapshot = Snapshot::new(content, record["metadata"].clone());
                artifacts
                    .entry(base_path)
                    .or_default()
                    .insert(version, snapshot);
            }
        }
    }

    // 3. Generate Timelines & Events
    let mut all_events = Vec::new();
    let mut all_timelines = Vec::new();

    for (name, versions) in artifacts {
        let sorted_versions: Vec<(&usize, &Snapshot)> = versions.iter().collect();
        let mut current_timeline_id = "main".to_string();

        // Initialize Root Timeline
        all_timelines.push(Timeline {
            artifact: name.clone(),
            id: current_timeline_id.clone(),
            parent_id: None,
            branch_point: None,
            reason: BranchReason::Root,
        });

        // Handle Creation (Version 0)
        if let Some((&0, first)) = sorted_versions.first() {
            all_events.push(EvolutionEvent {
                event_type: "creation".to_string(),
                artifact: name.clone(),
                timeline_id: current_timeline_id.clone(),
                version: Some(0),
                version_from: None,
                version_to: None,
                diff_summary: "Initial Creation".to_string(),
                full_content: first.content.clone(),
                timestamp: first.metadata.get("modified").cloned(),
            });
        }

        // Handle Evolution
        for window in sorted_versions.windows(2) {
            let (v_prev, prev) = window[0];
            let (v_curr, curr) = window[1];

            // Check for Timeline Split
            if detect_timelines {
                if let Some(reason) = detect_split(prev, curr) {
                    // Start new timeline
                    let new_timeline_id = format!("branch_v{}", v_curr);

                    all_timelines.push(Timeline {
                        artifact: name.clone(),
                        id: new_timeline_id.clone(),
                        parent_id: Some(current_timeline_id.clone()),
                        branch_point: Some(*v_prev),
                        reason,
                    });

                    current_timeline_id = new_timeline_id;
                }
            }

            // Generate Diff
            let diff = TextDiff::from_lines(&prev.content, &curr.content);
            let mut diff_summary = String::new();

            for change in diff.iter_all_changes() {
                match change.tag() {
                    ChangeTag::Delete => diff_summary.push_str(&format!("- {}", change)),
                    ChangeTag::Insert => diff_summary.push_str(&format!("+ {}", change)),
                    ChangeTag::Equal => {}
                }
            }

            if !diff_summary.is_empty() {
                all_events.push(EvolutionEvent {
                    event_type: "evolution".to_string(),
                    artifact: name.clone(),
                    timeline_id: current_timeline_id.clone(),
                    version: None,
                    version_from: Some(*v_prev),
                    version_to: Some(*v_curr),
                    diff_summary,
                    full_content: curr.content.clone(),
                    timestamp: curr.metadata.get("modified").cloned(),
                });
            }
        }
    }

    if detect_timelines {
        let analysis = TimelineAnalysis {
            timelines: all_timelines,
            events: all_events,
        };
        Ok(to_value(analysis)?)
    } else {
        // Legacy: Return flat list of events (serialized as Value::Array)
        // convert EvolutionEvent structs to Value to match legacy JSON structure expectations if needed
        // But since EvolutionEvent derives Serialize, to_value(all_events) works and produces array
        Ok(to_value(all_events)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct MockLoader {
        files: HashMap<String, String>,
    }

    impl MockLoader {
        fn new() -> Self {
            Self {
                files: HashMap::new(),
            }
        }
        fn add(&mut self, path: &str, content: &str) {
            self.files.insert(path.to_string(), content.to_string());
        }
    }

    impl SnapshotLoader for MockLoader {
        fn load_content(&self, path: &str) -> Option<String> {
            self.files.get(path).cloned()
        }
    }

    #[test]
    fn test_process_state_evolution_legacy() {
        let mut loader = MockLoader::new();
        loader.add("task.md.resolved.0", "Task 1");
        loader.add("task.md.resolved.1", "Task 1\nTask 2");

        let input = json!([
            {"metadata": {"path": "task.md.resolved.0"}},
            {"metadata": {"path": "task.md.resolved.1"}}
        ]);

        let result = process_state(input, &loader, false).unwrap();
        let events = result.as_array().unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["timeline_id"], "main");
    }

    #[test]
    fn test_massive_rewrite_detection() {
        let mut loader = MockLoader::new();
        // V0: 10 lines of content
        let content_v0 = "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10";
        // V1: Rewrite! Only 1 line shared, 9 deleted.
        let content_v1 = "line1\nNEW_A\nNEW_B\nNEW_C\nNEW_D\nNEW_E";

        loader.add("doc.md.resolved.0", content_v0);
        loader.add("doc.md.resolved.1", content_v1);

        let input = json!([
            {"metadata": {"path": "doc.md.resolved.0"}},
            {"metadata": {"path": "doc.md.resolved.1"}}
        ]);

        // Enable timeline detection
        let result = process_state(input, &loader, true).unwrap();

        // Should output an Object with "timelines" and "events"
        let analysis: TimelineAnalysis = serde_json::from_value(result).unwrap();

        assert_eq!(analysis.timelines.len(), 2);

        // Assert Main Timeline
        assert_eq!(analysis.timelines[0].id, "main");
        assert_eq!(analysis.timelines[0].reason, BranchReason::Root);

        // Assert Divergent Timeline
        let branch = &analysis.timelines[1];
        assert_eq!(branch.id, "branch_v1");
        assert_eq!(branch.parent_id.as_deref(), Some("main"));
        assert_eq!(branch.branch_point, Some(0));

        match &branch.reason {
            BranchReason::MassiveRewrite {
                deletion_ratio,
                overlap_ratio,
            } => {
                // 9 deleted out of 10 lines = 0.9 ratio
                assert!(*deletion_ratio > 0.8);
                // 1 shared out of 10 lines = 0.1 ratio
                assert!(*overlap_ratio < 0.2);
            }
            _ => panic!("Expected MassiveRewrite reason"),
        }
    }
}
