use clap::{Args, Subcommand};
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;
// removed

#[derive(Args, Debug)]
pub struct HistoryArgs {
    #[command(subcommand)]
    pub command: HistoryCommands,
}

#[derive(Subcommand, Debug)]
pub enum HistoryCommands {
    /// Ingest a specific version of a repository
    Ingest {
        /// Git reference (SHA, tag, branch)
        #[arg(long, short = 'r')]
        git_ref: String,

        /// Repository path (defaults to current dir)
        #[arg(default_value = ".")]
        path: String,

        /// Collection
        #[arg(long, short, default_value = "docs")]
        collection: String,
        // field removed
    },
}

pub async fn run(
    args: HistoryArgs,
    config: &Config,
    profile_name: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    match args.command {
        HistoryCommands::Ingest {
            git_ref,
            path,
            collection,
            ..
        } => {
            let resolution = config.resolve_with(profile_name, Some(&collection), overrides)?;

            let file_detector = Arc::new(HybridDetector::new());
            let parser_factory = Arc::new(VecqParserFactory);

            let services = vecdb_core::CoreServices::from_config(
                config,
                file_detector.clone(),
                parser_factory.clone(),
            );
            let core = vecdb_core::Core::new(&resolution, services).await?;

            if OUTPUT.is_interactive {
                println!(
                    "Time Traveling to: {} @ {} (Collection: {})",
                    path, git_ref, collection
                );
            }
            core.ingest_history(
                &path,
                &git_ref,
                &collection,
                512,
                resolution.quantization.clone(),
                None,
            )
            .await?;
        }
    }
    Ok(())
}
