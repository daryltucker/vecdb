use clap::{Args, Subcommand};
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;
// removed

/// Printed once per invocation. A feature that half works is more dangerous
/// than one that does not exist, because it is trusted.
const WIP_BANNER: &str = "\
warning: `vecdb history` is a WORK IN PROGRESS.

         Ingesting a named revision works. Retaining more than one revision of
         the same file does not: on the AST path used for code and markdown,
         commit_sha is not part of the chunk ID, so a second revision upserts
         over the first instead of sitting beside it.

         Do not rely on cross-revision retrieval yet.";

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

        /// Collection to write into. Required — see below.
        ///
        /// This defaulted to `"docs"`: the only CLI subcommand carrying an
        /// unprefixed, production-sounding collection name as a default. A
        /// mistyped or omitted `-c` silently created or wrote a real-looking
        /// collection instead of failing, which is the misroute the rest of the
        /// tool is built to refuse. Every test passed `-c` explicitly, so the
        /// default was never exercised and never noticed.
        ///
        /// No default. `ingest` resolves an omitted collection through the
        /// profile and `.vecdbrc`; until `history` does the same, naming it is
        /// the only honest option.
        #[arg(long, short)]
        collection: String,
    },
}

pub async fn run(
    args: HistoryArgs,
    config: &Config,
    profile_name: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    // Not gated on `is_interactive`: an agent driving this over --json is
    // exactly who must not assume cross-revision retrieval works.
    eprintln!("{WIP_BANNER}\n");

    match args.command {
        HistoryCommands::Ingest {
            git_ref,
            path,
            collection,
            ..
        } => {
            let resolution = config.resolve_with(profile_name, Some(&collection), overrides)?;

            let file_detector = Arc::new(HybridDetector::new());
            let parser_factory = Arc::new(VecqParserFactory::default());

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
                // The collection's own resolved chunking, not a literal.
                //
                // This was `512`, discarding the resolution computed three lines
                // above, so `history ingest` cut chunks at a granularity nothing
                // in config.toml or .vecdbrc mentioned — into the same collection
                // an ordinary `ingest` fills at the configured one.
                vecdb_core::ingestion::options::ChunkSpec {
                    target_chunk_size: resolution.target_chunk_size.value,
                    chunk_overlap: resolution.chunk_overlap.value,
                    // Clamped to the model's capacity, exactly as `ingest`
                    // does. Two commands filling one collection must agree.
                    max_chunk_bytes: Some(resolution.effective_max_chunk_bytes()),
                    pack_target_bytes: Some(resolution.pack_target_bytes.value),
                },
                resolution.quantization.clone(),
                None,
            )
            .await?;
        }
    }
    Ok(())
}
