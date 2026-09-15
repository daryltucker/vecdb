use clap::Subcommand;
use clap_complete::Shell;

pub mod config;
pub mod delete;
pub mod enable_usages;
pub mod history;
pub mod ingest;
pub mod list;
pub mod man;
pub mod optimize;
pub mod search;
pub mod snapshot;
pub mod status;

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize configuration
    Init,

    /// Recursively ingest documents from a path into a collection.
    Ingest(ingest::IngestArgs),

    /// Search the index
    Search(vecdb_core::tools::SearchArgs),

    /// List available collections
    List,

    /// Show system status and connectivity
    Status(status::StatusArgs),

    /// Delete a collection
    Delete(delete::DeleteArgs),

    /// Manage Collection Snapshots (Create, List, Download, Restore)
    Snapshot(snapshot::SnapshotArgs),

    /// Display manual
    Man(man::ManArgs),

    /// Manage config settings
    Config(config::ConfigArgs),

    /// Optimize a collection (apply quantization)
    Optimize(optimize::OptimizeArgs),

    /// [WIP] Time Travel / History Operations
    ///
    /// Ingesting a named revision works. Retaining more than one revision of
    /// the same file does not: the AST path seeds chunk IDs on
    /// `doc_id :: trail :: content_hash` with `doc_id` derived from the path
    /// alone, so a second revision upserts over the first. The generic chunker
    /// path does include `commit_sha` and behaves as intended.
    History(history::HistoryArgs),

    /// Enable usage/reference extraction mode
    EnableUsages(enable_usages::EnableUsagesArgs),

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}
