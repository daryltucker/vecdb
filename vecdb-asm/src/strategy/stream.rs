use anyhow::{bail, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// Processes the input JSON value (array) using the Stream Consolidation strategy.
///
/// 1. Expects a JSON Array.
/// 2. Deduplicates records based on content hash (SHA256 of JSON string).
/// 3. Sorts records by `metadata.modified` timestamp.
/// Processes the input JSON value (array) using the Stream Consolidation strategy.
///
/// 1. Expects a JSON Array.
/// 2. Deduplicates records based on content hash (SHA256 of JSON string).
/// 3. Sorts records by `metadata.modified` timestamp.
/// 4. If `stitch` is true, merges overlapping text fragments between sequential records.
pub fn process_stream(input: Value, no_dedupe: bool, stitch: bool) -> Result<Value> {
    // 1. Validate Input
    let records = match input {
        Value::Array(arr) => arr,
        _ => bail!("Stream strategy requires a JSON array as input (use vecq --slurp)"),
    };

    if records.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }

    // 2. Deduplicate
    let mut unique_records = Vec::new();
    let mut seen_hashes = HashSet::new();

    for record in records {
        if !no_dedupe {
            let hash = calculate_content_hash(&record);
            if seen_hashes.contains(&hash) {
                continue;
            }
            seen_hashes.insert(hash);
        }
        unique_records.push(record);
    }

    // 3. Sort by Timestamp
    unique_records.sort_by(|a, b| {
        let a_time = get_modified_time(a);
        let b_time = get_modified_time(b);
        a_time.cmp(&b_time)
    });

    if !stitch {
        return Ok(Value::Array(unique_records));
    }

    // 4. Overlap Stitching
    if unique_records.is_empty() {
        return Ok(Value::Array(Vec::new()));
    }

    let mut stitched = Vec::new();
    let mut current = unique_records[0].clone();

    for next in unique_records.iter().skip(1) {
        let current_text = current["content"].as_str().unwrap_or("");
        let next_text = next["content"].as_str().unwrap_or("");

        let merged = vecdb_common::stitch_text(current_text, next_text);

        // If merged length is less than sum of parts, we found an overlap
        if merged.len() < current_text.len() + next_text.len() {
            if let Some(obj) = current.as_object_mut() {
                obj.insert("content".to_string(), Value::String(merged));
                // Update timestamp to latest merged part
                if let Some(next_mod) = next.get("timestamp") {
                    obj.insert("timestamp".to_string(), next_mod.clone());
                }
            }
        } else {
            stitched.push(current);
            current = next.clone();
        }
    }
    stitched.push(current);

    Ok(Value::Array(stitched))
}

fn calculate_content_hash(val: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(val.to_string());
    let result = hasher.finalize();
    result
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

fn get_modified_time(val: &Value) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    // Try "timestamp" first (common in streams), then fall back to metadata.modified
    val.get("timestamp")
        .or_else(|| val.get("metadata").and_then(|m| m.get("modified")))
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_calculate_content_hash() {
        let val = json!({"key": "value"});
        let hash = calculate_content_hash(&val);
        assert!(!hash.is_empty());

        let val2 = json!({"key": "value"});
        let hash2 = calculate_content_hash(&val2);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_process_stream_deduplication() {
        let input = json!([
            {"id": 1, "content": "A", "metadata": {"modified": "2023-01-01T10:00:00Z"}},
            {"id": 2, "content": "B", "metadata": {"modified": "2023-01-01T11:00:00Z"}},
            {"id": 1, "content": "A", "metadata": {"modified": "2023-01-01T10:00:00Z"}}
        ]);

        let result = process_stream(input, false, false).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn test_process_stream_sorting() {
        let input = json!([
            {"id": 2, "content": "B", "metadata": {"modified": "2023-01-01T12:00:00Z"}},
            {"id": 1, "content": "A", "metadata": {"modified": "2023-01-01T10:00:00Z"}}
        ]);

        let result = process_stream(input, false, false).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[1]["id"], 2);
    }
}
