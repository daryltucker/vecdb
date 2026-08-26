// PURPOSE:
//   Project-level AST analysis using vecq's existing parse/graph infrastructure.
//
//   Walks a directory (respecting .vectorignore), parses each supported file, and
//   produces a JGF v2 architecture graph + Mermaid diagram using the graph_src and
//   graph_to_architecture jq normalizers already built into the query engine.
//
//   This is the canonical implementation. Both vecdb-server (MCP tool) and Ivaldi
//   (library dep) call this directly — there is no duplication.

use crate::error::{VecqError, VecqResult};
use crate::{convert_to_json, parse_file, query_json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vecdb_common::FileType;

/// Arguments for `project_overview`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOverviewArgs {
    /// Root directory to analyze.
    pub path: PathBuf,

    /// Maximum directory depth to recurse (default: 10).
    pub max_depth: Option<usize>,

    /// Additional ignore patterns on top of .vectorignore.
    #[serde(default)]
    pub ignore_patterns: Vec<String>,

    /// Whether to respect .gitignore files (default: false).
    /// `.vectorignore` is always respected unless `ignore_vectorignore` is true.
    #[serde(default = "default_respect_gitignore")]
    pub respect_gitignore: bool,

    /// If true, bypass `.vectorignore` during file walking (default: false).
    /// This is the vecdb standard pattern: default false = respect the ignore file,
    /// set true to opt out.
    #[serde(default)]
    pub ignore_vectorignore: bool,

    /// Whether to skip hidden files/directories (default: true).
    #[serde(default = "default_skip_hidden")]
    pub skip_hidden: bool,
}

fn default_respect_gitignore() -> bool {
    false
}

fn default_skip_hidden() -> bool {
    true
}

/// Result of a `project_overview` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOverview {
    /// Canonicalized root path that was analyzed.
    pub project_root: String,

    /// Number of files successfully parsed.
    pub files_analyzed: usize,

    /// Number of files skipped (binary, unsupported type, read error, parse error).
    pub files_skipped: usize,

    /// Architectural JGF v2 graph: files, modules, structs, classes, interfaces only.
    /// Pruned from the full symbol graph for readability. Use `code_query` for full detail.
    pub graph: serde_json::Value,

    /// Mermaid diagram generated from the architectural graph.
    pub mermaid: String,
}

/// Analyze a project directory: walk, parse, graph, render.
///
/// Respects `.vectorignore` via the `ignore` crate. `.gitignore` is opt-in
/// (default: false). Files that cannot be read or parsed are counted in
/// `files_skipped` without failing the overall analysis.
pub async fn project_overview(args: ProjectOverviewArgs) -> VecqResult<ProjectOverview> {
    let root = args
        .path
        .canonicalize()
        .map_err(|e| VecqError::ParseError {
            file: args.path.clone(),
            line: 0,
            message: format!("Cannot access path: {e}"),
            source: Some(Box::new(e)),
        })?;

    let max_depth = args.max_depth.unwrap_or(10);

    let mut builder = ignore::WalkBuilder::new(&root);
    builder.max_depth(Some(max_depth));
    builder.hidden(!args.skip_hidden);
    builder.git_ignore(args.respect_gitignore);
    builder.git_global(args.respect_gitignore);

    // .vectorignore is the primary noise filter — respected by default.
    // Use ignore_vectorignore: true to bypass (agent opt-out pattern).
    if !args.ignore_vectorignore {
        builder.add_custom_ignore_filename(".vectorignore");
    }

    for pattern in &args.ignore_patterns {
        builder.add_ignore(pattern);
    }

    let mut file_jsons: Vec<serde_json::Value> = Vec::new();
    let mut files_analyzed: usize = 0;
    let mut files_skipped: usize = 0;

    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                files_skipped += 1;
                continue;
            }
        };

        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let path_str = path.to_string_lossy();
        let file_type = FileType::from_path(&*path_str);

        if !file_type.is_supported() {
            files_skipped += 1;
            continue;
        }

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => {
                files_skipped += 1;
                continue;
            }
        };

        let parsed = match parse_file(&content, file_type).await {
            Ok(p) => p,
            Err(_) => {
                files_skipped += 1;
                continue;
            }
        };

        let mut json = match convert_to_json(parsed) {
            Ok(j) => j,
            Err(_) => {
                files_skipped += 1;
                continue;
            }
        };

        // graph_src.jq reads .metadata.path to label each file node.
        // Inject the real path since parse_file() doesn't receive a path argument.
        if let Some(meta) = json.get_mut("metadata").and_then(|m| m.as_object_mut()) {
            meta.insert(
                "path".to_string(),
                serde_json::Value::String(path.display().to_string()),
            );
        }

        file_jsons.push(json);
        files_analyzed += 1;
    }

    if file_jsons.is_empty() {
        return Ok(ProjectOverview {
            project_root: root.display().to_string(),
            files_analyzed: 0,
            files_skipped,
            graph: serde_json::json!({ "graphs": [] }),
            mermaid: String::new(),
        });
    }

    // Apply src_to_architecture (= src_to_graph | graph_to_architecture):
    // prunes down to files, modules, structs, classes, interfaces.
    let input = serde_json::Value::Array(file_jsons);
    let arch_results =
        query_json(&input, "src_to_architecture").map_err(|e| VecqError::ConfigError {
            message: format!("src_to_architecture failed: {e}"),
        })?;
    let graph = arch_results
        .into_iter()
        .next()
        .unwrap_or(serde_json::json!({ "graphs": [] }));

    // Render Mermaid from the architectural JGF v2 graph.
    let mermaid = query_json(&graph, "graph_format_mermaid")
        .ok()
        .and_then(|mut r| r.pop())
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();

    Ok(ProjectOverview {
        project_root: root.display().to_string(),
        files_analyzed,
        files_skipped,
        graph,
        mermaid,
    })
}
