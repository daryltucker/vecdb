use crate::ingestion::options::IngestionOptions;
use crate::parsers::ParserFactory;
use crate::types::Chunk;
use anyhow::Result;
use sha2::Digest;
use sha2::Sha256;
use std::path::Path;
use std::sync::Arc;
use vecdb_common::FileType;

pub struct TwoPassIngestor;

impl TwoPassIngestor {
    pub async fn process_large_file(
        path: &Path,
        rel_path: &Path,
        parser_factory: Arc<dyn ParserFactory>,
        _options: Arc<IngestionOptions>,
        file_type: FileType,
        metadata: Option<serde_json::Value>,
    ) -> Result<Vec<Chunk>> {
        let file_size = std::fs::metadata(path)?.len();
        let segment_size = 5 * 1024 * 1024; // 5MB
        let overlap_size = 512 * 1024; // 512KB

        let mut chunks = Vec::new();
        let mut offset = 0;

        let doc_id = rel_path.to_string_lossy().to_string();

        while offset < file_size {
            let read_size =
                std::cmp::min(segment_size + overlap_size, (file_size - offset) as usize);
            let mut buffer = vec![0u8; read_size];

            use std::io::Read;
            use std::io::Seek;
            let mut file = std::fs::File::open(path)?;
            file.seek(std::io::SeekFrom::Start(offset))?;
            file.read_exact(&mut buffer)?;

            let content = String::from_utf8_lossy(&buffer).to_string();

            // Pass 2: Parse Segment
            let segment_chunks = if let Some(p) = parser_factory.get_parser(file_type) {
                p.parse(&content, path, metadata.clone()).await?
            } else {
                // Fallback to basic processing
                let mut c = Vec::new();
                c.push(Chunk {
                    id: uuid::Uuid::new_v4().to_string(),
                    document_id: doc_id.clone(),
                    content: content.clone(),
                    vector: None,
                    metadata: metadata
                        .clone()
                        .unwrap_or(serde_json::json!({}))
                        .as_object()
                        .unwrap()
                        .clone()
                        .into_iter()
                        .collect(),
                    page_num: None,
                    char_start: offset as usize,
                    char_end: (offset as usize) + content.len(),
                    start_line: None,
                    end_line: None,
                });
                c
            };

            chunks.extend(segment_chunks);

            if offset + (segment_size as u64) >= file_size {
                break;
            }
            offset += segment_size as u64;
        }

        // Deduplicate chunks by content hash to handle overlaps
        let mut unique_chunks = Vec::new();
        let mut seen_hashes = std::collections::HashSet::new();

        for chunk in chunks {
            let hash = calculate_hash(&chunk.content);
            if !seen_hashes.contains(&hash) {
                seen_hashes.insert(hash);
                unique_chunks.push(chunk);
            }
        }

        Ok(unique_chunks)
    }
}

fn calculate_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    result
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}
