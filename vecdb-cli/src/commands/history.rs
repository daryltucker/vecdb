use crate::vecq_adapter::VecqParserFactory;
use clap::{Args, Subcommand};
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
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

pub async fn run(args: HistoryArgs, config: &Config, profile_name: &str) -> anyhow::Result<()> {
    match args.command {
        HistoryCommands::Ingest {
            git_ref,
            path,
            collection,
            ..
        } => {
            let profile = config.resolve_profile(Some(profile_name), Some(&collection))?;

            let file_detector = Arc::new(HybridDetector::new());
            let parser_factory = Arc::new(VecqParserFactory);

            let core = vecdb_core::Core::new(
                &profile.qdrant_url,
                &profile.ollama_url,
                &config.resolve_embedding_model(&profile),
                profile.accept_invalid_certs,
                &profile.embedder_type,
                Some(config.fastembed_cache_path.clone()),
                config.resolve_local_use_gpu(Some(&collection)),
                profile.qdrant_api_key.clone(),
                profile.ollama_api_key.clone(),
                config.smart_routing_keys.clone(),
                config.ingestion.path_rules.clone(),
                config.ingestion.max_concurrent_requests,
                config.resolve_gpu_batch_size(&profile, Some(collection.as_str())),
                profile.num_ctx,
                file_detector.clone(),
                parser_factory.clone(),
            )
            .await?;

            if OUTPUT.is_interactive {
                println!(
                    "Time Traveling to: {} @ {} (Collection: {})",
                    path, git_ref, profile.default_collection_name
                );
            }
            core.ingest_history(
                &path,
                &git_ref,
                &profile.default_collection_name,
                512,
                profile.quantization.clone(),
                None,
            )
            .await?;
        }
    }
    Ok(())
}
