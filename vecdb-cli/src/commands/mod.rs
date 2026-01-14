use clap::Subcommand;
use clap_complete::Shell;

pub mod delete;
pub mod man;
pub mod status;
pub mod ingest;
pub mod search;
pub mod list;
pub mod history;
pub mod config;
pub mod optimize;
pub mod snapshot;

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

    /// Time Travel / History Operations
    History(history::HistoryArgs),

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}
