use clap::{Args, Parser};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Tool: Semantic search against the vector index
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct SearchArgs {
    /// The semantic query to run against the vector database
    pub query: String,

    /// Profile to use for collection resolution (optional, uses server default if not specified)
    #[arg(long)]
    pub profile: Option<String>,

    /// The collection to search in. Use list_collections to discover what is available.
    #[arg(long, short)]
    pub collection: Option<String>,

    /// Output results as JSON
    #[arg(long)]
    #[serde(default)]
    pub json: bool,

    /// Use smart routing to detect facets (overrides default search).
    /// Defaults to false if not specified.
    #[arg(long)]
    #[serde(default)]
    pub smart: Option<bool>,

    /// Minimum similarity score threshold (0.0-1.0). Results below this are filtered out.
    #[arg(long)]
    #[serde(default)]
    pub min_score: Option<f64>,
}

/// Tool: Generate vectors from text
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct EmbedArgs {
    /// List of texts to generate embeddings for
    #[arg(long, short, num_args = 1..)]
    pub texts: Vec<String>,
}

/// Tool: Ingest a local file or directory
///
/// Ingest local file/directory into a collection. Chunks, embeds, and stores content.
///
/// Security: Requires server started with --allow-local-fs flag.
///
/// CRITICAL: You MUST use absolute paths. If you use relative paths (e.g. "./") from the
/// MCP server's working directory, you may inadvertently scan the entire home directory.
///
/// ADVISEMENT: If you are creating a temporary collection for research, please delete it
/// using `delete_collection` when you are finished to save disk space.
///
/// Example: ingest_path(path='/absolute/path/to/docs', collection='my-docs')
///
/// Workflow: Ingest -> list_collections (verify) -> search_vectors (query).
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct IngestPathArgs {
    /// The local path (file or directory) to ingest. MUST be an absolute path.
    #[arg(long, short)]
    pub path: String,

    /// Profile to use for collection resolution (optional, uses server default if not specified)
    #[arg(long)]
    pub profile: Option<String>,

    /// The target collection to ingest into. If it doesn't exist, it will be created.
    #[arg(long, short)]
    pub collection: Option<String>,

    /// Max concurrent file processing tasks (optional, uses server default if not specified)
    #[arg(long, short = 'c')]
    pub concurrency: Option<usize>,

    /// Max concurrent GPU embedding tasks (optional, uses server default if not specified)
    #[arg(long, short = 'G')]
    pub gpu_concurrency: Option<usize>,

    /// Ignore .vectorignore files during file walking
    #[arg(long, default_value_t = false)]
    #[serde(default)]
    pub ignore_vectorignore: bool,
}

/// Tool: Ingest a historic version of a repository
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct IngestHistoryArgs {
    /// Path to the repository (local path or URL)
    pub repo_path: String,

    /// Git reference to ingest (SHA, tag, branch)
    pub git_ref: String,

    /// Profile to use for collection resolution (optional, uses server default if not specified)
    #[arg(long)]
    pub profile: Option<String>,

    /// Target collection
    #[arg(long)]
    pub collection: Option<String>,
}

/// Tool: Query source code structure using vecq
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct VecqToolArgs {
    /// The jq-style query to run against the code structure (e.g. .functions[] | .name)
    pub query: String,

    /// Path to the file or directory to query
    pub path: String,

    /// Source type: 'local' (default) or 'git'
    #[arg(long)]
    pub source: Option<String>,

    /// Git reference (required if source='git')
    #[arg(long)]
    pub git_ref: Option<String>,

    /// Git repository path (required if source='git')
    #[arg(long)]
    pub repo_path: Option<String>,
}

/// Tool: Generate a structural overview of a project using AST analysis
///
/// Walks a directory (respecting .vectorignore), parses each supported source file,
/// and returns a JGF v2 architecture graph and Mermaid diagram.
///
/// Security: Requires server started with --allow-local-fs flag.
///
/// Example: project_overview(path='/absolute/path/to/project')
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct ProjectOverviewArgs {
    /// Absolute path to the project root directory
    pub path: String,

    /// Maximum directory depth to recurse (default: 10)
    #[arg(long)]
    pub max_depth: Option<usize>,

    /// Additional ignore patterns on top of .vectorignore (e.g. "*.generated.rs")
    #[arg(long, num_args = 0..)]
    #[serde(default)]
    pub ignore_patterns: Vec<String>,

    /// Whether to respect .gitignore files (default: false)
    #[arg(long)]
    pub respect_gitignore: Option<bool>,

    /// If true, bypass `.vectorignore` during file walking (default: false).
    /// This is the vecdb standard pattern: default false = respect the ignore file,
    /// set true to opt out for one-off overrides.
    #[arg(long)]
    pub ignore_vectorignore: Option<bool>,

    /// Whether to skip hidden files/directories (default: true)
    #[arg(long)]
    pub skip_hidden: Option<bool>,
}

/// Tool: Check status of background jobs
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct JobStatusArgs {
    /// Optional Job ID to filter for a specific task
    #[arg(long, short)]
    pub id: Option<String>,
}

/// Enum for easy CLI dispatch (Optional)
#[derive(Debug, Parser)]
pub enum ToolCommand {
    Search(SearchArgs),
    Embed(EmbedArgs),
    IngestPath(IngestPathArgs),
}
