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

    /// Enable `key:value` facet qualifiers in the query (e.g. "parse errors language:rust").
    /// Qualifiers are removed from the text before embedding, and the filters that
    /// were applied are reported back in the response. An unknown facet value is an
    /// error listing the valid ones, not a silently empty result set.
    /// Off by default; the query is searched exactly as written.
    #[arg(long, overrides_with = "no_smart")]
    #[serde(default)]
    pub smart: bool,

    /// Disable facet qualifier parsing. Present so the choice can be made
    /// explicitly on the command line when a config default turns `smart` on.
    #[arg(long = "no-smart", overrides_with = "smart")]
    #[serde(skip)]
    #[schemars(skip)]
    pub no_smart: bool,

    /// Maximum number of results to return. Defaults to 10.
    #[arg(long, short = 'n')]
    #[serde(default)]
    pub limit: Option<u64>,

    /// Minimum similarity score threshold (0.0-1.0). Applied by the vector store
    /// before the result limit is imposed, so a threshold never silently returns
    /// fewer results than exist above it.
    #[arg(long)]
    #[serde(default)]
    pub min_score: Option<f64>,
}

impl SearchArgs {
    /// Resolve the retrieval knobs into backend parameters.
    ///
    /// Single place where the defaults are applied, so the CLI and the MCP
    /// server cannot disagree about what `limit` or `min_score` mean.
    pub fn to_search_params(&self) -> crate::backend::SearchParams {
        crate::backend::SearchParams::new(self.limit.unwrap_or(crate::config::DEFAULT_SEARCH_LIMIT))
            .with_score_threshold(self.min_score.map(|s| s as f32))
    }

    /// Whether facet qualifier parsing is active for this request.
    pub fn use_smart(&self) -> bool {
        self.smart && !self.no_smart
    }
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

#[cfg(test)]
mod search_args_tests {
    use super::*;
    use crate::config::DEFAULT_SEARCH_LIMIT;

    /// Deserializing from the wire is how the MCP server builds these, so the
    /// tests exercise that path rather than constructing the struct by hand.
    fn from_json(v: serde_json::Value) -> SearchArgs {
        serde_json::from_value(v).expect("SearchArgs should deserialize")
    }

    #[test]
    fn minimal_request_gets_the_shared_default_limit() {
        let args = from_json(serde_json::json!({"query": "hello"}));
        assert_eq!(args.to_search_params().limit, DEFAULT_SEARCH_LIMIT);
        assert!(args.to_search_params().score_threshold.is_none());
        assert!(!args.use_smart());
    }

    #[test]
    fn limit_is_honored() {
        let args = from_json(serde_json::json!({"query": "hello", "limit": 50}));
        assert_eq!(args.to_search_params().limit, 50);
    }

    /// Regression: min_score used to be applied client-side after a hardcoded
    /// limit of 10. It must now reach the backend so the store can apply it
    /// before truncating.
    #[test]
    fn min_score_reaches_the_backend_params() {
        let args = from_json(serde_json::json!({"query": "hello", "min_score": 0.75}));
        let threshold = args.to_search_params().score_threshold.expect("threshold");
        assert!((threshold - 0.75).abs() < f32::EPSILON);
    }

    /// Regression: `smart` was `Option<bool>`, which produced an MCP schema with
    /// no declared default. Callers could not tell whether omitting it meant
    /// on or off. It is now a plain bool defaulting to false.
    #[test]
    fn smart_defaults_to_off_when_omitted() {
        assert!(!from_json(serde_json::json!({"query": "q"})).use_smart());
    }

    #[test]
    fn smart_can_be_turned_on_explicitly() {
        assert!(from_json(serde_json::json!({"query": "q", "smart": true})).use_smart());
    }

    #[test]
    fn schema_declares_the_smart_default_and_omits_it_from_required() {
        let schema = serde_json::to_value(schemars::schema_for!(SearchArgs)).unwrap();

        assert_eq!(
            schema["properties"]["smart"]["default"],
            serde_json::json!(false),
            "an agent must be able to read the default off the schema"
        );

        let required = schema["required"].as_array().cloned().unwrap_or_default();
        assert!(required.iter().any(|r| r == "query"));
        for optional in ["smart", "limit", "min_score", "collection"] {
            assert!(
                !required.iter().any(|r| r == optional),
                "{optional} must not be required"
            );
        }

        // `no_smart` is a command-line affordance only; leaking it into the tool
        // schema would offer a model two ways to say the same thing.
        assert!(schema["properties"].get("no_smart").is_none());
    }
}
