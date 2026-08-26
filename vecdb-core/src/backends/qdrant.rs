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
 *   3. Filter mapping: implemented in `json_to_qdrant_filter`
 *      Rationale: `qdrant::Filter` does not implement Deserialize, so payload
 *      filters arrive as generic JSON and are mapped by hand. Used by faceted
 *      search — see `router.rs`.
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
            .await
            .map_err(|e| {
                let err_str = e.to_string();
                if err_str.contains("404") || err_str.contains("grpc-status header missing") {
                    anyhow::anyhow!(
                        "Qdrant error (404): The server at this URL did not recognize the gRPC request.\n\
                         >> Possible causes: (1) hitting the REST port instead of gRPC, (2) missing /v1 prefix if proxied, (3) proxy misconfiguration.\n\
                         Original Error: {}",
                        err_str
                    )
                } else {
                    anyhow::anyhow!("Failed to create collection: {}", err_str)
                }
            })?;

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
        params: crate::backend::SearchParams,
    ) -> Result<Vec<SearchResult>> {
        let crate::backend::SearchParams {
            limit,
            filter,
            score_threshold,
        } = params;

        let qdrant_filter = filter.map(|f| self.json_to_qdrant_filter(f));

        // Over-fetch by one. The genesis point is a real point in the collection
        // and can score against any query, but it is bookkeeping rather than
        // content and is dropped below. Without the +1 a caller asking for
        // `limit` results would silently receive `limit - 1` whenever genesis
        // ranked in. There is exactly one genesis point per collection, so one
        // extra row is sufficient. Truncated back to `limit` after filtering.
        let fetch_limit = limit.saturating_add(1);

        let search_result = self
            .client
            .search_points(SearchPoints {
                collection_name: collection.to_string(),
                vector: vector.to_vec(),
                filter: qdrant_filter,
                limit: fetch_limit,
                // Applied by Qdrant during traversal, i.e. BEFORE the limit cut.
                // Filtering client-side after truncation would turn a threshold
                // into a silent result-count reduction.
                score_threshold,
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
            .filter_map(|scored_point| {
                let payload = scored_point.payload;

                // Skip genesis points
                if payload.get("type").and_then(|v| match &v.kind {
                    Some(Kind::StringValue(s)) => Some(s.as_str()),
                    _ => None,
                }) == Some("genesis")
                {
                    return None;
                }

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

                Some(SearchResult {
                    id: id_str,
                    score: scored_point.score,
                    content,
                    document_id,
                    metadata,
                })
            })
            // Give back the over-fetched slot: honor the caller's limit exactly.
            .take(limit as usize)
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

    async fn delete_stale_points(
        &self,
        collection: &str,
        document_id: &str,
        keep: &[String],
    ) -> Result<usize> {
        use qdrant_client::qdrant::{DeletePointsBuilder, PointsIdsList, ScrollPoints};
        use std::collections::HashSet;

        // Scroll the document's points and diff against `keep`, rather than
        // expressing "not in this id set" as a server-side filter. The id list
        // is per-file and small, the diff is exact, and it yields a real count
        // to report — a filter delete returns no indication of what it touched.
        let keep: HashSet<&str> = keep.iter().map(|s| s.as_str()).collect();
        let filter = Filter::must([Condition::from(FieldCondition {
            key: "document_id".to_string(),
            r#match: Some(Match {
                match_value: Some(MatchValue::Keyword(document_id.to_string())),
            }),
            ..Default::default()
        })]);

        let mut stale: Vec<PointId> = Vec::new();
        let mut offset = None;
        loop {
            let result = self
                .client
                .scroll(ScrollPoints {
                    collection_name: collection.to_string(),
                    filter: Some(filter.clone()),
                    with_payload: Some(false.into()),
                    with_vectors: Some(false.into()),
                    limit: Some(256),
                    offset: offset.clone(),
                    ..Default::default()
                })
                .await?;

            for point in &result.result {
                let id = match &point.id {
                    Some(PointId {
                        point_id_options: Some(PointIdOptions::Uuid(u)),
                    }) => u.clone(),
                    Some(PointId {
                        point_id_options: Some(PointIdOptions::Num(n)),
                    }) => n.to_string(),
                    _ => continue,
                };
                if !keep.contains(id.as_str()) {
                    stale.push(PointId::from(id));
                }
            }

            offset = result.next_page_offset;
            if offset.is_none() {
                break;
            }
        }

        if stale.is_empty() {
            return Ok(0);
        }

        let removed = stale.len();
        self.client
            .delete_points(
                DeletePointsBuilder::new(collection)
                    .points(PointsIdsList { ids: stale })
                    .wait(true),
            )
            .await?;

        Ok(removed)
    }

    async fn list_collections(&self) -> Result<Vec<String>> {
        let result = self.client.list_collections().await?;
        Ok(result.collections.into_iter().map(|c| c.name).collect())
    }

    async fn get_collection_info(&self, name: &str) -> Result<crate::types::CollectionInfo> {
        let info = self.client.collection_info(name).await?;

        let (vector_count, vector_size, quantization, vectors_on_disk, payload_on_disk) =
            if let Some(result) = info.result {
                let count = result.points_count;
                let (size, quant, vectors_on_disk_inner, payload_on_disk_inner) = result
                    .config
                    .map(|c| {
                        let s = c.params.clone().and_then(|p| {
                            p.vectors_config.and_then(|vc| match vc.config {
                                Some(qdrant_client::qdrant::vectors_config::Config::Params(vp)) => {
                                    Some((vp.size, vp.on_disk))
                                }
                                _ => None,
                            })
                        });

                        let q = c.quantization_config.and_then(|qc| {
                            qc.quantization.map(|q_enum| {
                                match q_enum {
                            qdrant_client::qdrant::quantization_config::Quantization::Scalar(_) => {
                                crate::config::QuantizationType::Scalar
                            }
                            qdrant_client::qdrant::quantization_config::Quantization::Binary(_) => {
                                crate::config::QuantizationType::Binary
                            }
                            qdrant_client::qdrant::quantization_config::Quantization::Product(
                                _,
                            ) => crate::config::QuantizationType::None, // Not supported
                        }
                            })
                        });

                        let payload_on_disk = c.params.map(|p| p.on_disk_payload);

                        match s {
                            Some((size_val, on_disk)) => {
                                (Some(size_val), q, on_disk, payload_on_disk)
                            }
                            None => (None, q, None, payload_on_disk),
                        }
                    })
                    .unwrap_or((None, None, None, None));

                (
                    count,
                    size,
                    quant,
                    vectors_on_disk_inner,
                    payload_on_disk_inner,
                )
            } else {
                (None, None, None, None, None)
            };

        Ok(crate::types::CollectionInfo {
            name: name.to_string(),
            vector_count,
            vector_size,
            quantization,
            vectors_on_disk,
            payload_on_disk,
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

    async fn write_genesis(
        &self,
        collection: &str,
        meta: &crate::types::GenesisMetadata,
    ) -> Result<()> {
        use qdrant_client::qdrant::UpsertPoints;

        let sv = |s: &str| Value {
            kind: Some(Kind::StringValue(s.to_string())),
        };
        let iv = |n: u64| Value {
            kind: Some(Kind::IntegerValue(n as i64)),
        };

        let mut payload = HashMap::new();
        payload.insert("type".to_string(), sv("genesis"));
        payload.insert(
            "__meta_collection_identity".to_string(),
            sv(&meta.collection_id),
        );
        payload.insert("__meta_embedder_model".to_string(), sv(&meta.model.name));
        payload.insert("__meta_dimension".to_string(), iv(meta.dimension));
        payload.insert("__meta_distance".to_string(), sv(&meta.distance));

        payload.insert("__meta_created_at".to_string(), sv(&meta.created_at));
        // The magic marker. Written first in spirit: everything else in this
        // payload is only meaningful if this key says the collection is ours.
        payload.insert(
            "__meta_vecdb".to_string(),
            sv(&crate::types::CollectionGenesis::marker_value()),
        );
        // Which BUILD, not just which release. Between releases the version is
        // constant while semantics change — the fidelity fix shipped inside
        // 1.0.4 — so the version alone cannot tell a pre-fix collection from a
        // post-fix one.
        payload.insert(
            "__meta_vecdb_revision".to_string(),
            sv(&crate::types::build_revision()),
        );

        // The compatibility-class fields. Absent ones are simply not written,
        // which read_genesis maps back to None and the guard treats as
        // "insufficient identity" — a refusal, never an assumption.
        for (key, val) in [
            ("__meta_embedder_digest", &meta.model.digest),
            ("__meta_architecture", &meta.model.architecture),
            ("__meta_family", &meta.model.family),
            ("__meta_parameter_size", &meta.model.parameter_size),
            ("__meta_quantization_level", &meta.model.quantization_level),
        ] {
            if let Some(v) = val {
                payload.insert(key.to_string(), sv(v));
            }
        }
        if let Some(v) = meta.model.embedding_length {
            payload.insert("__meta_embedding_length".to_string(), iv(v));
        }
        if let Some(v) = meta.model.context_length {
            payload.insert("__meta_context_length".to_string(), iv(v));
        }

        // Chunking is baked into the vectors as permanently as the model is,
        // and is recoverable from nowhere afterwards. Written as a group: a
        // partial record would let a reader compare one parameter while
        // silently ignoring another that also moved.
        if let Some(c) = &meta.chunking {
            payload.insert(
                "__meta_chunk_target".to_string(),
                iv(c.target_chunk_size as u64),
            );
            payload.insert(
                "__meta_chunk_overlap".to_string(),
                iv(c.chunk_overlap as u64),
            );
            payload.insert(
                "__meta_chunk_max_bytes".to_string(),
                iv(c.max_chunk_bytes as u64),
            );
            payload.insert("__meta_chunk_tokenizer".to_string(), sv(&c.tokenizer));
        }

        let point = PointStruct::new(
            PointId::from(Uuid::nil().to_string()),
            vec![0.0; meta.dimension as usize],
            payload,
        );

        self.client
            .upsert_points(UpsertPoints {
                collection_name: collection.to_string(),
                points: vec![point],
                ..Default::default()
            })
            .await?;

        Ok(())
    }

    async fn read_genesis(&self, collection: &str) -> Result<crate::types::CollectionGenesis> {
        use qdrant_client::qdrant::GetPoints;

        let response = self
            .client
            .get_points(GetPoints {
                collection_name: collection.to_string(),
                ids: vec![PointId::from(Uuid::nil().to_string())],
                with_vectors: Some(qdrant_client::qdrant::WithVectorsSelector::from(false)),
                with_payload: Some(qdrant_client::qdrant::WithPayloadSelector::from(true)),
                ..Default::default()
            })
            .await?;

        let Some(point) = response.result.first() else {
            // Nothing at the nil UUID: not a vecdb collection.
            return Ok(crate::types::CollectionGenesis::default());
        };

        let get_s = |key: &str| -> Option<String> {
            point.payload.get(key).and_then(|v| match &v.kind {
                Some(Kind::StringValue(s)) => Some(s.clone()),
                _ => None,
            })
        };
        let get_i = |key: &str| -> Option<u64> {
            point.payload.get(key).and_then(|v| match &v.kind {
                Some(Kind::IntegerValue(i)) => Some(*i as u64),
                _ => None,
            })
        };

        // Check the magic marker before reading anything else. A point can sit
        // at the nil UUID without being ours; only the marker settles it, and
        // an absent marker means every other field here is meaningless.
        let vecdb_version = crate::types::CollectionGenesis::parse_marker(get_s("__meta_vecdb"));
        if vecdb_version.is_none() {
            return Ok(crate::types::CollectionGenesis::default());
        }

        Ok(crate::types::CollectionGenesis {
            vecdb_version,
            vecdb_revision: get_s("__meta_vecdb_revision"),
            collection_id: get_s("__meta_collection_identity"),
            model: crate::types::ModelIdentity {
                name: get_s("__meta_embedder_model").unwrap_or_default(),
                digest: get_s("__meta_embedder_digest"),
                architecture: get_s("__meta_architecture"),
                family: get_s("__meta_family"),
                parameter_size: get_s("__meta_parameter_size"),
                quantization_level: get_s("__meta_quantization_level"),
                embedding_length: get_i("__meta_embedding_length"),
                context_length: get_i("__meta_context_length"),
            },
            dimension: get_i("__meta_dimension"),
            distance: get_s("__meta_distance"),
            // All-or-nothing, matching how it is written. A half-read record
            // would invite comparing one parameter while another moved unseen.
            chunking: match (
                get_i("__meta_chunk_target"),
                get_i("__meta_chunk_overlap"),
                get_i("__meta_chunk_max_bytes"),
                get_s("__meta_chunk_tokenizer"),
            ) {
                (Some(target), Some(overlap), Some(max_bytes), Some(tokenizer)) => {
                    Some(crate::types::ChunkingIdentity {
                        target_chunk_size: target as usize,
                        chunk_overlap: overlap as usize,
                        max_chunk_bytes: max_bytes as usize,
                        tokenizer,
                    })
                }
                _ => None,
            },
            created_at: get_s("__meta_created_at"),
        })
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
        // Refused, not empty.
        //
        // Qdrant exposes no general task enumeration — there is per-collection
        // `optimizer_status`, and nothing that lists work in flight. This used
        // to `Ok(Vec::new())` "to fix build", which made `vecdb status` print
        // "No active remote tasks" as a statement of fact derived from a value
        // that was never looked up. An empty list and an unanswerable question
        // are different things, and only one of them should be reported as
        // reassurance.
        anyhow::bail!(
            "the Qdrant backend cannot enumerate background tasks — Qdrant exposes \
             per-collection optimizer status, not a task list. Local ingest jobs are \
             tracked separately and are reported by `vecdb status` and `get_job_status`."
        )
    }
}
