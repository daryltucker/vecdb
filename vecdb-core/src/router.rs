/*
 * PURPOSE:
 *   Intelligent query router that dynamically discovers database facets
 *   and applies metadata filters to searches.
 *
 * RATIONALE:
 *   Avoids hardcoded string manipulation and enables self-healing metadata routing.
 */

use crate::backend::Backend;
// use crate::types::SearchResult;
use anyhow::Result;
use std::sync::Arc;
use serde_json::json;

pub struct DynamicRouter {
    backend: Arc<dyn Backend + Send + Sync>,
    facet_keys: Vec<String>,
}

impl DynamicRouter {
    pub fn new(backend: Arc<dyn Backend + Send + Sync>, facet_keys: Vec<String>) -> Self {
        Self {
            backend,
            facet_keys,
        }
    }

    /// Route a query by discovering facets for MULTIPLE keys and matching them.
    /// Returns a map of detected filters (key -> value) and the original query.
    pub async fn route(&self, collection: &str, query: &str) -> Result<(serde_json::Map<String, serde_json::Value>, String)> {
        let mut detected_filters = serde_json::Map::new();
        // Regex handles case insensitivity via (?i)

        // Iterate through all monitored keys (e.g., "version", "cuda", "language")
        for key in &self.facet_keys {
            // 1. Discover available values for this key
            let facets = self.backend.list_metadata_values(collection, key).await?;
            
            // 2. Search for these values in the query
            for facet in facets {
                // Securely escape the facet value to prevent regex injection (e.g. if version is "1.0+")
                let escaped_facet = regex::escape(&facet);
                // Pattern: Case-insensitive, Word Boundary start, value, Word Boundary end
                let pattern = format!(r"(?i)\b{}\b", escaped_facet);
                
                if let Ok(re) = regex::Regex::new(&pattern) {
                    if re.is_match(query) {
                        if crate::output::OUTPUT.is_interactive {
                            eprintln!("DynamicRouter: Detected {}={}", key, facet);
                        }
                        detected_filters.insert(key.clone(), json!(facet));
                        // Once we find a match for a key, we stop looking for other values of THAT key
                        break;
                    }
                }
            }
        }

        Ok((detected_filters, query.to_string()))
    }
}
