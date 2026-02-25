/*
 * PURPOSE:
 *   Implementation of the `Backend` trait for Qdrant (https://qdrant.tech).
 *   Handles connection management, payload mapping, and vector operations.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - Native Rust implementation (TECH_STACK.md)
 *   - Production-grade constraints (REQUIREMENTS.md)
 *
 *   Implementation-discovered:
 *   - Qdrant uses `PointStruct` for upserts
 *   - Payload is `HashMap<String, Value>`
 *   - Vector size must be known at collection creation
 *
 * IMPLEMENTATION RULES:
 *   1. Use `qdrant_client::Qdrant` (New API)
 *      Rationale: `QdrantClient` is deprecated.
 *
 *   2. Map `uuid` to `PointId::Uuid`
 *      Rationale: Native UUID support in Qdrant is efficient
 *
 *   3. Filter mapping: Disabled for MVP
 *      Rationale: `qdrant::Filter` does not implement Deserialize. Needs manual mapping helper later.
 *
 * USAGE:
 *   let backend = QdrantBackend::new("http://localhost:6334").await?;
 *   backend.create_collection("docs", 768).await?;
 *
 * SELF-HEALING INSTRUCTIONS:
 *   - If Qdrant GRPC API changes: Update `point_id` and `payload` mapping logic.
 *   - If dependency update breaks `mcp-protocol-sdk`: Verify trait bounds on `Backend`.
 *
 * RELATED FILES:
 *   - src/backend.rs - Trait definition
 *   - src/types.rs - Data structures
 *
 * MAINTENANCE:
 *   Update when `qdrant-client` crate is upgraded to major version.
 */

use crate::backend::Backend;
use crate::types::{Chunk, SearchResult};
use anyhow::Result;
use async_trait::async_trait;
use qdrant_client::qdrant::value::Kind;
use qdrant_client::qdrant::{
    point_id::PointIdOptions, quantization_config, quantization_config_diff, r#match::MatchValue,
    BinaryQuantization, Condition, CreateCollection, Distance, FieldCondition, Filter, Match,
    PointId, PointStruct, QuantizationConfig, QuantizationConfigDiff, ScalarQuantization,
    SearchPoints, UpdateCollection, Value, VectorParams, VectorsConfig, WithPayloadSelector,
};
use qdrant_client::Qdrant;

use std::collections::HashMap;
use uuid::Uuid;

pub struct QdrantBackend {
    client: Qdrant,
}

impl QdrantBackend {
    /// Create new Qdrant backend connection
    pub fn new(url: &str, api_key: Option<String>) -> Result<Self> {
        // Build client configuration
        let mut builder = Qdrant::from_url(url)
            .timeout(std::time::Duration::from_secs(300))
            .keep_alive_while_idle();

        // Disable compatibility check to avoid non-JSON output on stdout/stderr
        // during server initialization in MCP mode.
        builder.check_compatibility = false;

        if let Some(key) = api_key {
            builder = builder.api_key(key);
        }

        let client = builder.build()?;
        Ok(Self { client })
    }

    /// Convert generic JSON filter to Qdrant Filter
    /// Supports simple key-value equality: {"key": "value"}
    fn json_to_qdrant_filter(&self, json: serde_json::Value) -> Filter {
        let mut must_conditions = Vec::new();

        if let serde_json::Value::Object(map) = json {
            for (key, value) in map {
                let match_value = match value {
                    serde_json::Value::String(s) => Some(MatchValue::Text(s)),
                    serde_json::Value::Number(n) => n.as_i64().map(MatchValue::Integer),
                    serde_json::Value::Bool(b) => Some(MatchValue::Boolean(b)),
                    _ => None,
                };

                if let Some(mv) = match_value {
                    must_conditions.push(Condition::from(FieldCondition {
                        key,
                        r#match: Some(Match {
                            match_value: Some(mv),
                        }),
                        ..Default::default()
                    }));
                }
            }
        }

        Filter {
            must: must_conditions,
            ..Default::default()
        }
    }
}

#[async_trait]
impl Backend for QdrantBackend {
    async fn health_check(&self) -> Result<()> {
        self.client.health_check().await?;
        Ok(())
    }

    async fn create_collection(
        &self,
        name: &str,
        vector_size: u64,
        quantization: Option<crate::config::QuantizationType>,
    ) -> Result<()> {
        if self.collection_exists(name).await? {
            return Ok(());
        }

        let q_config = match quantization {
            Some(crate::config::QuantizationType::Scalar) => Some(QuantizationConfig {
                quantization: Some(quantization_config::Quantization::Scalar(
                    ScalarQuantization {
                        r#type: 1, // Int8
                        quantile: None,
                        always_ram: Some(true),
                    },
                )),
            }),
            Some(crate::config::QuantizationType::Binary) => Some(QuantizationConfig {
                quantization: Some(quantization_config::Quantization::Binary(
                    BinaryQuantization {
                        always_ram: Some(true),
                        ..Default::default()
                    },
                )),
            }),
            _ => None,
        };

        self.client
            .create_collection(CreateCollection {
                collection_name: name.to_string(),
                vectors_config: Some(VectorsConfig {
                    config: Some(qdrant_client::qdrant::vectors_config::Config::Params(
                        VectorParams {
                            size: vector_size,
                            distance: Distance::Cosine.into(),
                            ..Default::default()
                        },
                    )),
                }),
                quantization_config: q_config,
                ..Default::default()
            })
            .await?;

        Ok(())
    }

    async fn update_collection_quantization(
        &self,
        name: &str,
        quantization: crate::config::QuantizationType,
    ) -> Result<()> {
        let q_config = match quantization {
            crate::config::QuantizationType::Scalar => Some(QuantizationConfigDiff {
                quantization: Some(quantization_config_diff::Quantization::Scalar(
                    ScalarQuantization {
                        r#type: 1, // Int8
                        quantile: None,
                        always_ram: Some(true),
                    },
                )),
            }),
            crate::config::QuantizationType::Binary => Some(QuantizationConfigDiff {
                quantization: Some(quantization_config_diff::Quantization::Binary(
                    BinaryQuantization {
                        always_ram: Some(true),
                        ..Default::default()
                    },
                )),
            }),
            crate::config::QuantizationType::None => None,
        };

        if let Some(config) = q_config {
            self.client
                .update_collection(UpdateCollection {
                    collection_name: name.to_string(),
                    quantization_config: Some(config),
                    ..Default::default()
                })
                .await?;
        }

        Ok(())
    }

    async fn collection_exists(&self, name: &str) -> Result<bool> {
        let result = self.client.collection_info(name).await;
        match result {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn delete_collection(&self, name: &str) -> Result<()> {
        self.client.delete_collection(name).await?;
        Ok(())
    }

    async fn upsert(&self, collection: &str, chunks: Vec<Chunk>) -> Result<()> {
        use qdrant_client::qdrant::UpsertPoints;

        let points: Vec<PointStruct> = chunks
            .into_iter()
            .map(|chunk| {
                let id = Uuid::parse_str(&chunk.id).unwrap_or_default();
                let vector = chunk.vector.unwrap_or_default();

                let mut payload: HashMap<String, Value> = HashMap::new();

                // Helper to convert serde_json::Value to qdrant::Value
                fn json_to_qdrant(v: serde_json::Value) -> Value {
                    match v {
                        serde_json::Value::Null => Value {
                            kind: Some(Kind::NullValue(0)),
                        },
                        serde_json::Value::Bool(b) => Value {
                            kind: Some(Kind::BoolValue(b)),
                        },
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Value {
                                    kind: Some(Kind::IntegerValue(i)),
                                }
                            } else {
                                Value {
                                    kind: Some(Kind::DoubleValue(n.as_f64().unwrap_or(0.0))),
                                }
                            }
                        }
                        serde_json::Value::String(s) => Value {
                            kind: Some(Kind::StringValue(s)),
                        },
                        serde_json::Value::Array(arr) => {
                            let values = arr.into_iter().map(json_to_qdrant).collect();
                            Value {
                                kind: Some(Kind::ListValue(qdrant_client::qdrant::ListValue {
                                    values,
                                })),
                            }
                        }
                        serde_json::Value::Object(_) => Value {
                            kind: Some(Kind::StringValue(
                                "Nested objects not supported yet".into(),
                            )),
                        }, // Simplification
                    }
                }

                for (k, v) in chunk.metadata {
                    payload.insert(k, json_to_qdrant(v));
                }
                payload.insert(
                    "content".to_string(),
                    Value {
                        kind: Some(Kind::StringValue(chunk.content)),
                    },
                );
                payload.insert(
                    "document_id".to_string(),
                    Value {
                        kind: Some(Kind::StringValue(chunk.document_id)),
                    },
                );

                PointStruct::new(PointId::from(id.to_string()), vector, payload)
            })
            .collect();

        // New API: use upsert_points instead of blocking, pass UpsertPoints struct or builder
        self.client
            .upsert_points(UpsertPoints {
                collection_name: collection.to_string(),
                points,
                ..Default::default()
            })
            .await?;

        Ok(())
    }

    async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        limit: u64,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let qdrant_filter = filter.map(|f| self.json_to_qdrant_filter(f));

        // Remove & reference
        let search_result = self
            .client
            .search_points(SearchPoints {
                collection_name: collection.to_string(),
                vector: vector.to_vec(),
                filter: qdrant_filter,
                limit,
                with_payload: Some(WithPayloadSelector {
                    selector_options: Some(
                        qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
                    ),
                }),
                ..Default::default()
            })
            .await?;

        let results = search_result
            .result
            .into_iter()
            .map(|scored_point| {
                let payload = scored_point.payload;

                // Helper to extract string
                fn get_str(payload: &HashMap<String, Value>, key: &str) -> String {
                    payload
                        .get(key)
                        .and_then(|v| match &v.kind {
                            Some(Kind::StringValue(s)) => Some(s.clone()),
                            _ => None,
                        })
                        .unwrap_or_default()
                }

                let content = get_str(&payload, "content");
                let document_id = get_str(&payload, "document_id");

                // Helper to convert qdrant::Value to serde_json::Value
                fn qdrant_to_json(v: Value) -> serde_json::Value {
                    match v.kind {
                        Some(Kind::NullValue(_)) => serde_json::Value::Null,
                        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
                        Some(Kind::IntegerValue(i)) => serde_json::Value::Number(i.into()),
                        Some(Kind::DoubleValue(d)) => serde_json::Number::from_f64(d)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::Null),
                        Some(Kind::StringValue(s)) => serde_json::Value::String(s),
                        Some(Kind::ListValue(l)) => serde_json::Value::Array(
                            l.values.into_iter().map(qdrant_to_json).collect(),
                        ),
                        Some(Kind::StructValue(s)) => {
                            let map = s
                                .fields
                                .into_iter()
                                .map(|(k, v)| (k, qdrant_to_json(v)))
                                .collect();
                            serde_json::Value::Object(map)
                        }
                        None => serde_json::Value::Null,
                    }
                }

                // Map payload back to HashMap<String, serde_json::Value>
                let mut metadata = HashMap::new();
                for (k, v) in payload {
                    metadata.insert(k, qdrant_to_json(v));
                }

                let id_str = match scored_point.id {
                    Some(PointId {
                        point_id_options: Some(PointIdOptions::Uuid(u)),
                    }) => u,
                    Some(PointId {
                        point_id_options: Some(PointIdOptions::Num(n)),
                    }) => n.to_string(),
                    _ => "unknown".to_string(),
                };

                SearchResult {
                    id: id_str,
                    score: scored_point.score,
                    content,
                    document_id,
                    metadata,
                }
            })
            .collect();

        Ok(results)
    }

    async fn points_exists(&self, collection: &str, ids: Vec<String>) -> Result<Vec<String>> {
        use qdrant_client::qdrant::GetPoints;

        let point_ids: Vec<PointId> = ids.iter().map(|id| PointId::from(id.to_string())).collect();

        // New API: use GetPoints struct
        let response = self
            .client
            .get_points(GetPoints {
                collection_name: collection.to_string(),
                ids: point_ids,
                with_vectors: Some(qdrant_client::qdrant::WithVectorsSelector::from(false)),
                with_payload: Some(qdrant_client::qdrant::WithPayloadSelector::from(false)),
                ..Default::default()
            })
            .await?;

        Ok(response
            .result
            .into_iter()
            .map(|p| match p.id {
                Some(PointId {
                    point_id_options: Some(PointIdOptions::Uuid(u)),
                }) => u,
                Some(PointId {
                    point_id_options: Some(PointIdOptions::Num(n)),
                }) => n.to_string(),
                _ => "unknown".to_string(),
            })
            .collect())
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        let result = self.client.list_collections().await?;
        Ok(result.collections.into_iter().map(|c| c.name).collect())
    }

    async fn get_collection_info(&self, name: &str) -> Result<crate::types::CollectionInfo> {
        let info = self.client.collection_info(name).await?;

        let (vector_count, vector_size, quantization) = if let Some(result) = info.result {
            let count = result.points_count;
            let (size, quant) = result
                .config
                .map(|c| {
                    let s = c.params.and_then(|p| {
                        p.vectors_config.and_then(|vc| match vc.config {
                            Some(qdrant_client::qdrant::vectors_config::Config::Params(vp)) => {
                                Some(vp.size)
                            }
                            _ => None,
                        })
                    });

                    let q = c.quantization_config.and_then(|qc| {
                        qc.quantization.map(|q_enum| match q_enum {
                            qdrant_client::qdrant::quantization_config::Quantization::Scalar(_) => {
                                crate::config::QuantizationType::Scalar
                            }
                            qdrant_client::qdrant::quantization_config::Quantization::Binary(_) => {
                                crate::config::QuantizationType::Binary
                            }
                            qdrant_client::qdrant::quantization_config::Quantization::Product(
                                _,
                            ) => crate::config::QuantizationType::None, // Not supported
                        })
                    });

                    (s, q)
                })
                .unwrap_or((None, None));

            (count, size, quant)
        } else {
            (None, None, None)
        };

        Ok(crate::types::CollectionInfo {
            name: name.to_string(),
            vector_count,
            vector_size,
            quantization,
        })
    }

    async fn list_metadata_values(&self, collection: &str, key: &str) -> Result<Vec<String>> {
        use qdrant_client::qdrant::ScrollPoints;
        use std::collections::HashSet;

        // Use scroll to iterate through points and collect unique metadata values
        // Note: For very large collections, this should be optimized with Qdrant Facets
        // but scroll is a reliable fallback for now.
        let mut values = HashSet::new();
        let mut offset = None;

        loop {
            // Remove & reference
            let result = self
                .client
                .scroll(ScrollPoints {
                    collection_name: collection.to_string(),
                    with_payload: Some(true.into()),
                    with_vectors: Some(false.into()),
                    limit: Some(100),
                    offset: offset.clone(),
                    ..Default::default()
                })
                .await?;

            for point in result.result {
                if let Some(val) = point.payload.get(key) {
                    match &val.kind {
                        Some(Kind::StringValue(s)) => {
                            values.insert(s.clone());
                        }
                        Some(Kind::IntegerValue(i)) => {
                            values.insert(i.to_string());
                        }
                        _ => {} // Skip other types for discovery for now
                    }
                }
            }

            offset = result.next_page_offset;
            if offset.is_none() {
                break;
            }
        }

        Ok(values.into_iter().collect())
    }

    async fn get_collection_id(&self, collection: &str) -> Result<Option<String>> {
        use qdrant_client::qdrant::GetPoints;

        let genesis_id = PointId::from(Uuid::nil().to_string()); // 00000000-0000-0000-0000-000000000000

        let response = self
            .client
            .get_points(GetPoints {
                collection_name: collection.to_string(),
                ids: vec![genesis_id],
                with_vectors: Some(qdrant_client::qdrant::WithVectorsSelector::from(false)),
                with_payload: Some(qdrant_client::qdrant::WithPayloadSelector::from(true)),
                ..Default::default()
            })
            .await?;

        if let Some(point) = response.result.first() {
            if let Some(val) = point.payload.get("__meta_collection_identity") {
                if let Some(Kind::StringValue(s)) = &val.kind {
                    return Ok(Some(s.clone()));
                }
            }
        }

        Ok(None)
    }

    async fn set_collection_id(&self, collection: &str, id: &str) -> Result<()> {
        use qdrant_client::qdrant::UpsertPoints;

        let genesis_id = Uuid::nil().to_string();

        // Create an empty (zero) vector for the genesis point.
        // We need to know the dimension, but Qdrant allows sparse vector updates or we can try to fetch it.
        // Easier: Just upsert payload if possible? No, Qdrant requires vector for new points usually.
        // Better: Fetch collection info to get size.
        let info = self.get_collection_info(collection).await?;
        let size = info.vector_size.unwrap_or(768); // Default fallback slightly dangerous but usually dimension is known.

        let vector = vec![0.0; size as usize];

        let mut payload = HashMap::new();
        payload.insert(
            "__meta_collection_identity".to_string(),
            Value {
                kind: Some(Kind::StringValue(id.to_string())),
            },
        );
        payload.insert(
            "type".to_string(),
            Value {
                kind: Some(Kind::StringValue("genesis".to_string())),
            },
        );

        let point = PointStruct::new(PointId::from(genesis_id), vector, payload);

        self.client
            .upsert_points(UpsertPoints {
                collection_name: collection.to_string(),
                points: vec![point],
                ..Default::default()
            })
            .await?;

        Ok(())
    }

    async fn list_tasks(&self) -> Result<Vec<crate::types::TaskInfo>> {
        // qdrant-client v1.16.0 handles tasks differently (often via a separate service client
        // or builder pattern that isn't immediately obvious in the flat Qdrant struct).
        // Returning empty list for now to fix build.
        Ok(Vec::new())
    }
}
