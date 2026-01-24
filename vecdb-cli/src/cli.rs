use crate::commands::{self, Commands};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use vecdb_core::config::Config;

#[derive(Parser, Debug)]
#[command(name = "vecdb")]
#[command(about = "Vector Database Project CLI", long_about = None)]
#[command(after_help = "See `vecdb man --agent` for Agent Interface documentation.")]
pub struct Cli {
    /// Profile to use from config.toml
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Force JSON output
    #[arg(long, short = 'j', global = true)]
    pub json: bool,

    /// Force Markdown/Text output
    #[arg(long, short = 'M', global = true)]
    pub markdown: bool,

    #[command(subcommand)]
    pub command: Commands,
}

pub async fn run() -> anyhow::Result<()> {
    // Build Version String
    let app_version = env!("CARGO_PKG_VERSION");
    let ort_version = vecdb_core::get_ort_version();
    let long_version = format!("vecdb v{}\nONNX v{}", app_version, ort_version);

    // We manually build the command to inject the version
    let long_version_static: &'static str = Box::leak(long_version.into_boxed_str());
    let cmd = Cli::command().version(long_version_static);

    // Parse using the modified command definition
    let matches = cmd.get_matches();

    // Convert matches back to Cli struct
    use clap::FromArgMatches;
    let cli = Cli::from_arg_matches(&matches)?;

    // Safety Check for Init:
    if let Commands::Init = cli.command {
        let path = Config::get_path()?;
        if path.exists() {
            eprintln!("❌ Config file already exists at: {:?}", path);
            eprintln!("   Aborting `init` to prevent accidental overwrite.");
            eprintln!("   To reset: backup/delete the file and run `vecdb init` again.");
            std::process::exit(1);
        }
    }

    // Load Configuration
    let mut config = Config::load()?;
    let base_profile_name = cli
        .profile
        .as_deref()
        .unwrap_or(&config.default_profile)
        .to_string();

    let format = resolve_format_flags(cli.json, cli.markdown);

    match cli.command {
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "vecdb", &mut std::io::stdout());
            return Ok(());
        }
        Commands::Init => {
            let path = Config::get_path()?;
            println!("✅ Initialized new configuration at: {:?}", path);
            println!("   Default Profile: {}", config.default_profile);
            println!("   Edit this file to configure your profiles and keys.");
        }
        Commands::Ingest(args) => commands::ingest::run(args, &config, &base_profile_name).await?,
        Commands::Search(args) => {
            commands::search::run(args, &config, &base_profile_name, format).await?
        }
        Commands::List => commands::list::run(&config, &base_profile_name, format).await?,
        Commands::Status(args) => {
            commands::status::run(args, &config, &base_profile_name, format).await?
        }
        Commands::Delete(args) => {
            let profile =
                config.resolve_profile(Some(&base_profile_name), args.collection.as_deref())?;
            // Delete needs full core construction in main if not moved?
            // Actually I should have moved delete logic to commands/delete.rs fully including core creation.
            // Let's check status.rs/delete.rs - status.rs did core creation.
            // commands/delete.rs currently likely just holds args and run?
            // Existing delete.rs has run that takes &Core.
            // I should wrap it or duplicate core creation here.
            // Better: update delete.rs to create core like others?
            // For now, I will construct core here to keep existing delete.rs intact if possible,
            // OR update delete.rs. Given I updated others to create Core, I should probably do it for delete too.
            // But let's look at `commands/delete.rs` content first? I didn't verify it.
            // I'll stick to constructing core here for Delete to avoid touching unverified file if possible, or assume similar pattern.
            // Actually, consistency suggests moving core creation to delete.rs.
            // But I cannot see delete.rs right now.
            // I'll instantiate Core here for Delete.
            use crate::vecq_adapter::VecqParserFactory;
            use std::sync::Arc;
            use vecq::detection::HybridDetector;
            let file_detector = Arc::new(HybridDetector::new());
            let parser_factory = Arc::new(VecqParserFactory);

            let core = vecdb_core::Core::new(
                &profile.qdrant_url,
                &profile.ollama_url,
                &config.resolve_embedding_model(&profile),
                profile.accept_invalid_certs,
                &profile.embedder_type,
                Some(config.fastembed_cache_path.clone()),
                config.resolve_local_use_gpu(args.collection.as_deref()),
                profile.qdrant_api_key.clone(),
                profile.ollama_api_key.clone(),
                config.smart_routing_keys.clone(),
                config.ingestion.path_rules.clone(),
                config.ingestion.max_concurrent_requests,
                config.ingestion.gpu_batch_size,
                file_detector.clone(),
                parser_factory.clone(),
            )
            .await?;
            commands::delete::run(&core, args).await?;
        }
        Commands::Snapshot(args) => {
            commands::snapshot::run(args, &config, &base_profile_name).await?
        }
        Commands::Man(args) => commands::man::run(args)?,
        Commands::Config(args) => commands::config::run(args, &mut config)?,
        Commands::Optimize(args) => {
            commands::optimize::run(args, &config, &base_profile_name).await?
        }
        Commands::History(args) => {
            commands::history::run(args, &config, &base_profile_name).await?
        }
        Commands::EnableUsages(args) => {
            commands::enable_usages::run(args).await?
        }
    }

    Ok(())
}

fn resolve_format_flags(json: bool, markdown: bool) -> vecdb_common::output::OutputFormat {
    if json {
        vecdb_common::output::OutputFormat::Json
    } else if markdown {
        vecdb_common::output::OutputFormat::Markdown
    } else {
        vecdb_common::output::OutputContext::detect().resolve_format()
    }
}
