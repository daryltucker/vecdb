
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

    /// The collection to search in. Use 'list_collections' to discover what is available.
    #[arg(long, short)]
    pub collection: Option<String>,

    /// Output results as JSON
    #[arg(long)]
    pub json: bool,

    /// Use smart routing to detect facets (overrides default search)
    #[arg(long)]
    pub smart: bool,
}

/// Tool: Generate vectors from text
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct EmbedArgs {
    /// List of texts to generate embeddings for
    #[arg(long, short, num_args = 1..)]
    pub texts: Vec<String>,
}

/// Tool: Ingest a local file or directory
#[derive(Debug, Args, Serialize, Deserialize, JsonSchema, Clone)]
pub struct IngestPathArgs {
    /// The local path (file or directory) to ingest
    #[arg(long, short)]
    pub path: String,

    /// Profile to use for collection resolution (optional, uses server default if not specified)
    #[arg(long)]
    pub profile: Option<String>,

    /// The target collection to ingest into. If it doesn't exist, it will be created.
    #[arg(long, short)]
    pub collection: Option<String>,
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

/// Enum for easy CLI dispatch (Optional)
#[derive(Debug, Parser)]
pub enum ToolCommand {
    Search(SearchArgs),
    Embed(EmbedArgs),
    IngestPath(IngestPathArgs),
}
