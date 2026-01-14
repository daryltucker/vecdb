use anyhow::{Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use serde::Serialize;
use ignore::gitignore::GitignoreBuilder;
use ignore::WalkBuilder;
use std::collections::{HashMap, HashSet};

// Tree node for JSON serialization
#[derive(Serialize)]
struct TreeNode {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<TreeNode>>,
}

pub async fn gather_tree_context(dir: &Path) -> Result<(String, String)> {
    let root_path = dir.canonicalize().unwrap_or(dir.to_path_buf());

    // 1. Build walker respecting .vectorignore
    let walker = WalkBuilder::new(&root_path)
        .standard_filters(true) 
        .hidden(true) // Ignore hidden files (dotfiles)
        .max_depth(Some(3)) // Limit depth to avoid massive trees (similar to tree -L 2)
        .add_custom_ignore_filename(".vectorignore")
        .build();

    // 2. Collect valid paths and stats
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut extension_counts: HashMap<String, usize> = HashMap::new();
    
    for result in walker {
        match result {
            Ok(entry) => {
                let path = entry.path();
                // Skip .git explicitly
                if path.components().any(|c| c.as_os_str() == ".git") {
                    continue;
                }
                
                if path.is_file() {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        *extension_counts.entry(ext.to_lowercase()).or_insert(0) += 1;
                    }
                }

                if path.strip_prefix(&root_path).is_ok() {
                    paths.push(path.to_path_buf());
                }
            }
            Err(_) => continue,
        }
    }
    paths.sort();

    // Determine dominant language
    let mut top_lang = "Unknown".to_string();
    let mut max_count = 0;
    
    for (ext, count) in &extension_counts {
        if *count > max_count {
            max_count = *count;
            top_lang = match ext.as_str() {
                "rs" => "Rust",
                "py" => "Python",
                "js" => "JavaScript",
                "ts" => "TypeScript",
                "go" => "Go",
                "c" | "h" => "C",
                "cpp" | "hpp" | "cc" => "C++",
                "md" => "Markdown",
                "json" => "JSON",
                "toml" => "TOML",
                "sh" => "Shell",
                _ => continue,
            }.to_string();
        }
    }

    // 3. Build Tree Structure
    #[derive(Clone)]
    struct BuilderNode {
        name: String,
        is_dir: bool,
        children: HashMap<String, BuilderNode>,
    }

    let mut root = BuilderNode {
        name: root_path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        is_dir: true,
        children: HashMap::new(),
    };

    if paths.is_empty() {
        return Ok(("[]".to_string(), top_lang));
    }

    for path in paths {
        if path == root_path { continue; }
        let relative = path.strip_prefix(&root_path)?;
        let mut current = &mut root;
        
        for component in relative.components() {
            let name = component.as_os_str().to_string_lossy().to_string();
            current = current.children.entry(name.clone()).or_insert_with(|| BuilderNode {
                name,
                is_dir: path.is_dir(),
                children: HashMap::new(),
            });
            if path.ends_with(component) {
                current.is_dir = path.is_dir();
            }
        }
    }

    fn to_tree_node(node: BuilderNode) -> TreeNode {
        let mut children: Vec<TreeNode> = node.children.into_values()
            .map(to_tree_node)
            .collect();
        
        children.sort_by(|a, b| {
            match (a.entry_type == "directory", b.entry_type == "directory") {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });

        TreeNode {
            name: node.name,
            entry_type: if node.is_dir { "directory".to_string() } else { "file".to_string() },
            children: if node.is_dir { Some(children) } else { None },
        }
    }

    let tree_root = to_tree_node(root);
    let json_output = serde_json::to_string(&vec![tree_root])?;
    
    let final_json = if json_output.len() > 4096 { 
         let partial: String = json_output.chars().take(4096).collect();
         format!("{}...\n(truncated tree)", partial)
    } else {
        json_output
    };

    Ok((final_json, top_lang))
}

pub async fn read_readme(dir: &Path) -> Result<String> {
    let mut readme_path = dir.join("README.md");
    if !readme_path.exists() {
        readme_path = dir.join("readme.md");
    }
    if !readme_path.exists() {
        readme_path = dir.join("README.txt"); 
    }
    
    if readme_path.exists() {
        let file = std::fs::File::open(readme_path)?;
        let mut handle = file.take(2048);
        let mut buffer = String::new();
        handle.read_to_string(&mut buffer)?;
        if buffer.len() == 2048 {
            buffer.push_str("\n... (truncated) ...");
        }
        Ok(buffer)
    } else {
        Ok("No README found.".to_string())
    }
}

pub async fn call_vecdb_server(query: &str, dir: &Path, debug_mode: bool, smart: bool) -> Result<String> {
    // Build Sanitizer
    let mut builder = GitignoreBuilder::new(dir);
    builder.add(dir.join(".gitignore"));
    builder.add(dir.join(".vectorignore"));
    let sanitizer = builder.build()?;
    let mut private_counter = 1;

    let mut child = Command::new("vecdb-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("Failed to spawn vecdb-server")?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "docsize", "version": "0.1.0" }
        },
        "id": 1
    });
    let req_str = serde_json::to_string(&init_req)?;
    stdin.write_all(req_str.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), reader.next_line()).await;
    
    let search_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "name": "search_vectors",
            "arguments": {
                "query": query,
                "smart": smart,
                "json": true
            }
        },
        "id": 2
    });
    let search_str = serde_json::to_string(&search_req)?;
    stdin.write_all(search_str.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    
    let timeout_duration = std::time::Duration::from_secs(10);
    let start = std::time::Instant::now();
    let mut response_line = String::new();

    loop {
        if start.elapsed() > timeout_duration {
            if debug_mode { eprintln!("DEBUG: Timed out waiting for vecdb-server response"); }
            break;
        }

        match tokio::time::timeout(std::time::Duration::from_secs(2), reader.next_line()).await {
             Ok(Ok(Some(line))) => {
                 if debug_mode { eprintln!("DEBUG: Server: {}", line); }
                 if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&line) {
                     if resp.get("id").and_then(|id| id.as_i64()) == Some(2) {
                         response_line = line;
                         break;
                     }
                 }
             }
             Ok(Ok(None)) | Ok(Err(_)) => break,
             Err(_) => continue,
        }
    }

    if response_line.is_empty() {
        return Ok("No response from vector database details.".to_string());
    }

    let mut results_str = String::new();
    if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&response_line) {
        if let Some(result) = resp.get("result") {
             if let Some(content_array) = result.get("content").and_then(|v| v.as_array()) {
                 if let Some(text_val) = content_array.first().and_then(|c| c.get("text")) {
                     if let Some(inner_json_str) = text_val.as_str() {
                         let hits: Vec<serde_json::Value> = serde_json::from_str(inner_json_str).unwrap_or_default();
                         
                         let mut seen_paths = HashSet::new();
                         
                         for item in hits {
                             if let Some(raw_path) = item.get("metadata").and_then(|m| m.get("path")).and_then(|p| p.as_str()) {
                                 if seen_paths.insert(raw_path.to_string()) {
                                     // Check sanitization
                                     let is_ignored = sanitizer.matched(raw_path, false).is_ignore();
                                     
                                     let display_path = if is_ignored {
                                         let masked = format!("PRIVATE_KNOWLEDGE_SOURCE_{}", private_counter);
                                         private_counter += 1;
                                         if debug_mode { eprintln!("Sanitized {} -> {}", raw_path, masked); }
                                         masked
                                     } else {
                                         raw_path.to_string()
                                     };

                                     match contextualize_file(raw_path, dir, &display_path).await {
                                         Ok(content) => {
                                             results_str.push_str(&content);
                                             results_str.push_str("\n\n");
                                             if debug_mode { eprintln!("Contextualized {}", raw_path); }
                                         },
                                         Err(e) => {
                                             if debug_mode { eprintln!("Failed to contextualize {}: {}", raw_path, e); }
                                             let score = item.get("score").and_then(|s| s.as_f64()).unwrap_or(0.0);
                                             let snippet = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                             results_str.push_str(&format!("// Path: {} (Raw Snippet, Score: {:.2})\n{}\n\n", display_path, score, snippet));
                                         }
                                     }
                                 }
                             }
                         }
                     }
                 }
             }
        }
    }
    
    if results_str.is_empty() {
        Ok("No relevant context found.".to_string())
    } else {
        Ok(results_str)
    }
}

// Renamed and upgraded SKELETONIZE -> CONTEXTUALIZE
async fn contextualize_file(path_str: &str, start_dir: &Path, display_path: &str) -> Result<String> {
    let path = start_dir.join(path_str);
    if !path.exists() {
        return Ok(format!("// File not found: {}", display_path));
    }
    
    let content = std::fs::read_to_string(&path)?;
    
    // We simply return the full content now, as element parsing caused
    // missing data (gaps between elements) and duplication (nested elements).
    // The user wants "Full Body", so we give them the Full Body.
    
    Ok(format!("// Path: {} (Full Content)\n{}\n", display_path, content))
}

