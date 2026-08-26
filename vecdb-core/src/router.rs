/*
 * PURPOSE:
 *   Parses explicit `key:value` facet qualifiers out of a search query and
 *   turns them into backend metadata filters.
 *
 * RATIONALE:
 *   Scoping a search is a decision the caller makes, so it must be written in
 *   the query, not inferred from it. An earlier version of this router scanned
 *   the query for any bare word that happened to equal a known facet value —
 *   so "how do I parse rust files" silently became `language = rust`, and the
 *   caller had no way to see it or turn it off. Deterministic qualifiers are
 *   both predictable for an agent to emit and visible in the response.
 *
 * RELATED FILES:
 *   - src/lib.rs - Core::search_smart, which reports the applied filters back
 *   - src/config.rs - smart_routing_keys, the set of recognized qualifier keys
 */

use crate::backend::Backend;
use anyhow::Result;
use serde_json::json;
use std::sync::Arc;

/// Outcome of parsing a query for facet qualifiers.
#[derive(Debug, Clone, Default)]
pub struct RoutedQuery {
    /// Filters extracted from `key:value` qualifiers, ready for the backend.
    pub filters: serde_json::Map<String, serde_json::Value>,

    /// The query with all recognized qualifiers removed. This is what gets
    /// embedded — leaving `language:rust` in the text would pollute the vector
    /// with tokens the caller meant as metadata, not as meaning.
    pub query: String,
}

impl RoutedQuery {
    pub fn filter(&self) -> Option<serde_json::Value> {
        if self.filters.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(self.filters.clone()))
        }
    }
}

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

    /// Split a query into facet qualifiers and residual text.
    ///
    /// A token is a qualifier when it is exactly `key:value`, `key` is one of
    /// the configured `facet_keys` (case-insensitive), and `value` is non-empty.
    /// Everything else — including `http://`, `foo:bar` for an unconfigured
    /// `foo`, and a bare `rust` — is left in the query untouched.
    ///
    /// Returns `(filters, residual_query)` without consulting the backend, so
    /// it is cheap and unit-testable in isolation.
    fn parse_qualifiers(
        &self,
        query: &str,
    ) -> (serde_json::Map<String, serde_json::Value>, String) {
        let mut filters = serde_json::Map::new();
        let mut residual: Vec<&str> = Vec::new();

        for token in query.split_whitespace() {
            // split_once, not splitn/rsplit: `a:b:c` has value "b:c", which is a
            // legitimate payload value (e.g. a URL) rather than a parse error.
            let parsed = token.split_once(':').and_then(|(key, value)| {
                if value.is_empty() {
                    return None;
                }
                // Match the configured key case-insensitively but store the
                // configured spelling, since that is the actual payload key.
                self.facet_keys
                    .iter()
                    .find(|k| k.eq_ignore_ascii_case(key))
                    .map(|k| (k.clone(), value.to_string()))
            });

            match parsed {
                // First qualifier for a key wins; a repeated key is a caller
                // error we surface rather than silently overwrite.
                Some((key, value)) if !filters.contains_key(&key) => {
                    filters.insert(key, json!(value));
                }
                Some(_) => residual.push(token),
                None => residual.push(token),
            }
        }

        (filters, residual.join(" "))
    }

    /// Parse qualifiers and validate each value against the facet values that
    /// actually exist in the collection.
    ///
    /// Validation is not politeness. A qualifier naming a value that is not
    /// present matches nothing, and an unvalidated search would return an empty
    /// list — indistinguishable from "this collection has no answer." An agent
    /// reading that concludes the corpus is empty and stops. Naming the valid
    /// values instead turns a dead end into a retry it can act on.
    pub async fn route(&self, collection: &str, query: &str) -> Result<RoutedQuery> {
        let (filters, residual) = self.parse_qualifiers(query);

        if filters.is_empty() {
            // No qualifiers: zero backend round-trips, and the query is
            // unchanged. This is the overwhelmingly common path.
            return Ok(RoutedQuery {
                filters,
                query: query.to_string(),
            });
        }

        for (key, value) in &filters {
            let requested = value.as_str().unwrap_or_default();
            let available = self.backend.list_metadata_values(collection, key).await?;

            if !available.iter().any(|v| v == requested) {
                let mut sorted = available;
                sorted.sort();
                let shown: Vec<String> = sorted.iter().take(20).cloned().collect();
                let elided = sorted.len().saturating_sub(shown.len());

                return Err(anyhow::anyhow!(
                    "no such value for facet '{key}' in collection '{collection}': {requested:?}\n\
                     \n\
                     available: {}{}\n\
                     \n\
                     drop the '{key}:{requested}' qualifier to search unfiltered.",
                    if shown.is_empty() {
                        "(none — this collection has no points carrying that key)".to_string()
                    } else {
                        shown.join(", ")
                    },
                    if elided > 0 {
                        format!(" (+{elided} more)")
                    } else {
                        String::new()
                    },
                ));
            }
        }

        Ok(RoutedQuery {
            filters,
            query: residual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Chunk, CollectionInfo, SearchResult};
    use async_trait::async_trait;

    struct StubBackend;

    #[async_trait]
    impl Backend for StubBackend {
        async fn health_check(&self) -> Result<()> {
            Ok(())
        }
        async fn create_collection(
            &self,
            _n: &str,
            _s: u64,
            _q: Option<crate::config::QuantizationType>,
        ) -> Result<()> {
            Ok(())
        }
        async fn update_collection_quantization(
            &self,
            _n: &str,
            _q: crate::config::QuantizationType,
        ) -> Result<()> {
            Ok(())
        }
        async fn collection_exists(&self, _n: &str) -> Result<bool> {
            Ok(true)
        }
        async fn delete_collection(&self, _n: &str) -> Result<()> {
            Ok(())
        }
        async fn upsert(&self, _c: &str, _ch: Vec<Chunk>) -> Result<()> {
            Ok(())
        }
        async fn search(
            &self,
            _c: &str,
            _v: &[f32],
            _p: crate::backend::SearchParams,
        ) -> Result<Vec<SearchResult>> {
            Ok(vec![])
        }
        async fn points_exists(&self, _c: &str, _i: Vec<String>) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn delete_stale_points(&self, _c: &str, _d: &str, _k: &[String]) -> Result<usize> {
            Ok(0)
        }
        async fn list_collections(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn get_collection_info(&self, _n: &str) -> Result<CollectionInfo> {
            anyhow::bail!("not implemented in stub")
        }
        async fn list_metadata_values(&self, _c: &str, key: &str) -> Result<Vec<String>> {
            Ok(match key {
                "language" => vec!["rust".into(), "python".into()],
                "source_type" => vec!["code".into(), "docs".into()],
                _ => vec![],
            })
        }
        async fn set_collection_id(&self, _c: &str, _id: &str) -> Result<()> {
            Ok(())
        }
        async fn get_collection_id(&self, _c: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn list_tasks(&self) -> Result<Vec<crate::types::TaskInfo>> {
            Ok(vec![])
        }
        async fn write_genesis(&self, _c: &str, _m: &crate::types::GenesisMetadata) -> Result<()> {
            Ok(())
        }
        async fn read_genesis(&self, _c: &str) -> Result<crate::types::CollectionGenesis> {
            Ok(crate::types::CollectionGenesis::default())
        }
    }

    fn router() -> DynamicRouter {
        DynamicRouter::new(
            Arc::new(StubBackend),
            vec!["source_type".to_string(), "language".to_string()],
        )
    }

    /// The regression this module exists to prevent: a bare facet value in
    /// ordinary prose must not scope the search.
    #[test]
    fn bare_facet_word_is_not_a_filter() {
        let (filters, residual) = router().parse_qualifiers("how do I parse rust files");
        assert!(filters.is_empty());
        assert_eq!(residual, "how do I parse rust files");
    }

    #[test]
    fn qualifier_is_extracted_and_stripped() {
        let (filters, residual) = router().parse_qualifiers("parse files language:rust");
        assert_eq!(filters.get("language").unwrap(), "rust");
        assert_eq!(residual, "parse files");
    }

    #[test]
    fn qualifier_key_matching_is_case_insensitive_but_stores_configured_spelling() {
        let (filters, _) = router().parse_qualifiers("Language:rust");
        assert_eq!(filters.get("language").unwrap(), "rust");
    }

    #[test]
    fn unconfigured_key_is_left_in_the_query() {
        let (filters, residual) = router().parse_qualifiers("author:daryl notes");
        assert!(filters.is_empty());
        assert_eq!(residual, "author:daryl notes");
    }

    #[test]
    fn url_is_not_mistaken_for_a_qualifier() {
        let q = "see https://example.com/docs for setup";
        let (filters, residual) = router().parse_qualifiers(q);
        assert!(filters.is_empty());
        assert_eq!(residual, q);
    }

    #[test]
    fn multiple_distinct_keys_are_all_extracted() {
        let (filters, residual) =
            router().parse_qualifiers("language:rust source_type:code async runtime");
        assert_eq!(filters.len(), 2);
        assert_eq!(residual, "async runtime");
    }

    #[test]
    fn empty_value_is_not_a_qualifier() {
        let (filters, residual) = router().parse_qualifiers("language: rust");
        assert!(filters.is_empty());
        assert_eq!(residual, "language: rust");
    }

    #[tokio::test]
    async fn unknown_facet_value_errors_with_the_valid_values() {
        let err = router()
            .route("c", "language:cobol parse")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cobol"), "{err}");
        assert!(err.contains("rust"), "{err}");
        assert!(err.contains("python"), "{err}");
    }

    #[tokio::test]
    async fn unqualified_query_makes_no_backend_call_and_is_unchanged() {
        let routed = router()
            .route("c", "how do I parse rust files")
            .await
            .unwrap();
        assert!(routed.filter().is_none());
        assert_eq!(routed.query, "how do I parse rust files");
    }

    #[tokio::test]
    async fn valid_qualifier_survives_validation() {
        let routed = router().route("c", "language:rust parse").await.unwrap();
        assert_eq!(routed.query, "parse");
        assert_eq!(routed.filters.get("language").unwrap(), "rust");
    }
}
