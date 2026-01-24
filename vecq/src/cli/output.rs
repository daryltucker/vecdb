// Output formatting and processing for vecq CLI
// Handles JSON processing, file path injection, and result formatting

use std::io::Write;
use std::path::Path;
use tokio::fs;
use vecq::{detect_file_type, parse_file_with_options, FileType, FormatOptions, JqQueryEngine, JsonConverter, QueryEngine, UnifiedJsonConverter, VecqError, VecqResult};

use super::args::ParseOptions;

/// Extract JSON value(s) from a given input path (file, directory, or stdin)
pub async fn extract_json_from_input(
    path: &Path,
    options: &ParseOptions,
) -> VecqResult<Vec<serde_json::Value>> {
    if path.to_str() == Some("-") {
        use tokio::io::AsyncReadExt;
        let mut buffer = Vec::new();
        tokio::io::stdin()
            .read_to_end(&mut buffer)
            .await
            .map_err(|e| {
                VecqError::IoError(std::io::Error::new(
                    e.kind(),
                    format!("Failed to read from stdin: {}", e),
                ))
            })?;

        if !FileType::is_likely_text(&buffer) {
            return Err(VecqError::CircuitBreakerTriggered {
                message: "Stdin content appears to be binary or malformed text".to_string(),
            });
        }

        let content = String::from_utf8_lossy(&buffer);
        let vals = parse_content_to_json(&content, None, options).await?;
        Ok(vals)
    } else if path.is_file() {
        let vals = parse_file_to_json(path, options).await?;
        Ok(vals)
    } else if path.is_dir() {
        extract_json_from_directory(path, options, 0).await
    } else {
        Err(VecqError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Input path does not exist: {}", path.display()),
        )))
    }
}

async fn extract_json_from_directory(
    dir_path: &Path,
    options: &ParseOptions,
    current_depth: usize,
) -> VecqResult<Vec<serde_json::Value>> {
    let mut entries = fs::read_dir(dir_path).await.map_err(|e| {
        VecqError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to read directory {}: {}", dir_path.display(), e),
        ))
    })?;

    let mut values = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(VecqError::IoError)? {
        let path = entry.path();

        if path.is_dir() {
            if options.recursive {
                if let Some(max) = options.max_depth {
                    if current_depth >= max {
                        continue;
                    }
                }
                values.extend(
                    Box::pin(extract_json_from_directory(
                        &path,
                        options,
                        current_depth + 1,
                    ))
                    .await?,
                );
            }
            continue;
        }

        // Check if file type is supported
        let file_type = options
            .file_type
            .unwrap_or_else(|| detect_file_type(path.to_str().unwrap_or("")));
        if file_type == FileType::Unknown {
            continue;
        }

        match parse_file_to_json(&path, options).await {
            Ok(vals) => values.extend(vals),
            Err(_) if !options.verbose => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(values)
}

async fn parse_file_to_json(
    path: &Path,
    options: &ParseOptions,
) -> VecqResult<Vec<serde_json::Value>> {
    let content_bytes = fs::read(path).await.map_err(|e| {
        VecqError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to read file {}: {}", path.display(), e),
        ))
    })?;

    if !FileType::is_likely_text(&content_bytes) {
        return Err(VecqError::CircuitBreakerTriggered {
            message: format!(
                "File {} appears to be binary or malformed text",
                path.display()
            ),
        });
    }

    let content = String::from_utf8_lossy(&content_bytes);
    parse_content_to_json(&content, Some(path), options).await
}

async fn parse_content_to_json(
    content: &str,
    path: Option<&Path>,
    options: &ParseOptions,
) -> VecqResult<Vec<serde_json::Value>> {
    let file_type = if let Some(p) = path {
        options
            .file_type
            .unwrap_or_else(|| detect_file_type(p.to_str().unwrap_or("")))
    } else {
        options.file_type.unwrap_or(FileType::Unknown)
    };

    let mut json_vals = if file_type == FileType::Text {
        // Treat content as raw string, wrapped in single JSON string
        vec![serde_json::Value::String(content.to_string())]
    } else if file_type == FileType::Json
        || (file_type == FileType::Unknown
            && (content.trim_start().starts_with('{') || content.trim_start().starts_with('[')))
    {
        let deserializer = serde_json::Deserializer::from_str(content);
        let mut vals = Vec::new();
        for item in deserializer.into_iter::<serde_json::Value>() {
            match item {
                Ok(val) => vals.push(val),
                Err(e) => {
                    if file_type == FileType::Json {
                        return Err(VecqError::json_error(
                            "Invalid JSON input".to_string(),
                            Some(e),
                        ));
                    } else {
                        return Err(VecqError::UnsupportedFileType {
                            file_type: "Unknown (failed JSON heuristic)".to_string(),
                        });
                    }
                }
            }
        }
        vals
    } else {
        if file_type == FileType::Unknown {
            return Err(VecqError::UnsupportedFileType {
                file_type: format!("Unknown file type for: {:?}", path),
            });
        }
        let parsed = if options.enable_usages {
            parse_file_with_options(content, file_type, true).await?
        } else {
            vecq::parse_file(content, file_type).await?
        };
        let converter =
            UnifiedJsonConverter::with_default_schemas().with_context_lines(options.context_lines);
        vec![converter.convert(parsed)?]
    };

    // Inject Path into Metadata and recursively into all object nodes
    if let Some(p) = path {
        let path_str = p.to_string_lossy().to_string();
        for val in &mut json_vals {
            inject_file_path_recursive(val, &path_str);
        }
    }

    Ok(json_vals)
}

fn inject_file_path_recursive(value: &mut serde_json::Value, path: &str) {
    match value {
        serde_json::Value::Object(map) => {
            // Inject into metadata if present (root node usually)
            if let Some(metadata) = map.get_mut("metadata") {
                if let Some(meta_obj) = metadata.as_object_mut() {
                    meta_obj.insert(
                        "path".to_string(),
                        serde_json::Value::String(path.to_string()),
                    );

                }
            }

            // Inject into attributes if present, or create it if it looks like a document element
            let is_element = map.contains_key("element_type")
                || map.contains_key("kind")
                || map.contains_key("type");

            if is_element {
                if let Some(attributes) = map.get_mut("attributes") {
                    if let Some(attr_obj) = attributes.as_object_mut() {
                        attr_obj.insert(
                            "file_path".to_string(),
                            serde_json::Value::String(path.to_string()),
                        );
                    }
                } else {
                    let mut attr_obj = serde_json::Map::new();
                    attr_obj.insert(
                        "file_path".to_string(),
                        serde_json::Value::String(path.to_string()),
                    );
                    map.insert(
                        "attributes".to_string(),
                        serde_json::Value::Object(attr_obj),
                    );
                }
            }

            // check for "metadata" at root level if we are at root
            if !is_element && map.contains_key("elements") {
                // likely root document
                if !map.contains_key("metadata") {
                    let mut meta_obj = serde_json::Map::new();
                    meta_obj.insert(
                        "path".to_string(),
                        serde_json::Value::String(path.to_string()),
                    );
                    map.insert("metadata".to_string(), serde_json::Value::Object(meta_obj));
                }
            }

            // Recurse into all fields
            for (_, v) in map.iter_mut() {
                inject_file_path_recursive(v, path);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                inject_file_path_recursive(v, path);
            }
        }
        _ => {}
    }
}

// Synchronous version to support StdoutLock
pub fn process_json_value(
    json_value: serde_json::Value,
    query: &str,
    engine: &JqQueryEngine,
    output_format: &str,
    format_options: &FormatOptions,
    writer: &mut impl std::io::Write,
) -> VecqResult<()> {
    // Execute query if provided
    if !query.is_empty() {
        let results = engine.execute_query(&json_value, query)?;

        for result in results {
            let output = vecq::format_results(&result, output_format, format_options)?;
            if !output.is_empty() {
                if let Err(e) = writeln!(writer, "{}", output) {
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        std::process::exit(0);
                    }
                    return Err(VecqError::IoError(e));
                }
            }
        }
    } else {
        // Default: output formatted JSON
        let output = if format_options.pretty_print {
            serde_json::to_string_pretty(&json_value)?
        } else {
            serde_json::to_string(&json_value)?
        };

        if let Err(e) = writeln!(writer, "{}", output) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
            return Err(VecqError::IoError(e));
        }
    };

    Ok(())
}