use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::{HashMap, BTreeMap};
use regex::Regex;
use similar::{ChangeTag, TextDiff};
use crate::types::Snapshot;


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

/// Processes the input JSON value using the State Reduction strategy.
///
/// 1. Expects a JSON Array of records (from `vecq --slurp`).
/// 2. Groups records by artifact base name (e.g. `task.md` from `task.md.resolved.1`).
/// 3. Reads full content using the `loader`.
/// 4. Generates evolution events based on semantic diffs.
pub fn process_state(
    input: Value, 
    loader: &impl SnapshotLoader
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
    // Regex matches files ending in .resolved.<number>
    let re = Regex::new(r"(.+?)\.resolved\.(\d+)$").unwrap();

    for record in records {
        let path_str = record.get("metadata")
            .and_then(|m| m.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        
        if let Some(caps) = re.captures(path_str) {
            let base_path = caps.get(1).unwrap().as_str().to_string();
            let version_str = caps.get(2).unwrap().as_str();

            if let Ok(version) = version_str.parse::<usize>() {
                // Load content using the loader (abstraction)
                let content = loader.load_content(path_str).unwrap_or_default();
                
                let snapshot = Snapshot::new(
                    content,
                    record["metadata"].clone(),
                );

                artifacts.entry(base_path)
                    .or_default()
                    .insert(version, snapshot);
            }
        }
    }

    // 3. Generate Semantic Diffs
    let mut evolution_events = Vec::new();

    for (name, versions) in artifacts {
        let sorted_versions: Vec<(&usize, &Snapshot)> = versions.iter().collect();
        
        // Handle Creation (Version 0)
        if let Some((&0, first)) = sorted_versions.first() {
            evolution_events.push(json!({
                "event_type": "creation",
                "artifact": name,
                "version": 0,
                "diff_summary": "Initial Creation",
                "full_content": first.content, 
                "timestamp": first.metadata.get("modified").unwrap_or(&json!(null))
            }));
        }

        // Handle Evolution (Diff between N and N+1)
        for window in sorted_versions.windows(2) {
            let (v_prev, prev) = window[0];
            let (v_curr, curr) = window[1];
            
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
                evolution_events.push(json!({
                    "event_type": "evolution",
                    "artifact": name,
                    "version_from": v_prev,
                    "version_to": v_curr,
                    "diff_summary": diff_summary,
                    "full_content": curr.content,
                    "timestamp": curr.metadata.get("modified").unwrap_or(&json!(null))
                }));
            }
        }
    }

    Ok(Value::Array(evolution_events))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockLoader {
        files: HashMap<String, String>,
    }
    
    impl MockLoader {
        fn new() -> Self {
            Self { files: HashMap::new() }
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
    fn test_process_state_evolution() {
        let mut loader = MockLoader::new();
        loader.add("task.md.resolved.0", "Task 1");
        loader.add("task.md.resolved.1", "Task 1\nTask 2");

        let input = json!([
            {
                "metadata": {"path": "task.md.resolved.0", "modified": "2023-01-01T10:00:00Z"}
            },
            {
                "metadata": {"path": "task.md.resolved.1", "modified": "2023-01-01T11:00:00Z"}
            }
        ]);

        let result = process_state(input, &loader).unwrap();
        let events = result.as_array().unwrap();
        
        // Should have Creation + Evolution
        assert_eq!(events.len(), 2);
        
        assert_eq!(events[0]["event_type"], "creation");
        assert_eq!(events[0]["version"], 0);
        
        assert_eq!(events[1]["event_type"], "evolution");
        assert_eq!(events[1]["version_from"], 0);
        assert_eq!(events[1]["version_to"], 1);
    }
}
