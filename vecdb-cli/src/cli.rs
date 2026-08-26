use crate::commands::{self, Commands};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use vecdb_core::config::Config;

#[derive(Parser, Debug)]
#[command(name = "vecdb")]
#[command(about = "Vector Database Project CLI", long_about = None)]
#[command(after_help = "See `vecdb man --agent` for Agent Interface documentation.")]
pub struct Cli {
    // One flag per config layer: WHICH (profile) / WHAT (embedder) / WHERE
    // (backend). Overriding a layer with a flag beats redefining it in config,
    // which is how two definitions of the same embedder drift apart.
    /// Profile to use from config.toml — WHICH embedder and store
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Override the profile's embedder — WHAT model, and how it is tuned
    #[arg(long, global = true)]
    pub embedder: Option<String>,

    /// Run the resolved embedder on a different backend — WHERE it executes.
    /// Same model and tuning; only the host changes.
    #[arg(long, global = true)]
    pub backend: Option<String>,

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

    // Stamped at build time by vecdb-common/build.rs. Reading it from `git` at
    // runtime reported whatever was checked out then, not what this binary is.
    let git_hash = vecdb_common::revision();

    let ort_version = vecdb_core::get_ort_version();
    let long_version = format!(
        "vecdb v{} (git:{})\nONNX v{}",
        app_version, git_hash, ort_version
    );

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
    let profile_arg = cli.profile.as_deref();
    let overrides = vecdb_core::config::Overrides {
        embedder: cli.embedder.as_deref(),
        backend: cli.backend.as_deref(),
    };

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
        Commands::Ingest(args) => {
            commands::ingest::run(args, &config, profile_arg, overrides).await?
        }
        Commands::Search(args) => {
            commands::search::run(args, &config, profile_arg, overrides, format).await?
        }
        Commands::List => commands::list::run(&config, profile_arg, format).await?,
        Commands::Status(args) => {
            commands::status::run(args, &config, profile_arg, overrides, format).await?
        }
        Commands::Delete(args) => {
            let resolution =
                config.resolve_with(profile_arg, args.collection.as_deref(), overrides)?;

            if args.all {
                let is_local = resolution.qdrant_url.contains("localhost")
                    || resolution.qdrant_url.contains("127.0.0.1")
                    || resolution.qdrant_url.contains("0.0.0.0");
                if !is_local {
                    anyhow::bail!(
                        "Bulk deletion (--all) is restricted to local backends to prevent accidental data loss on remote systems ({}). \
                        To delete a remote collection, please specify it by name.",
                        resolution.qdrant_url
                    );
                }
            }

            // Delete only needs the backend (Qdrant) — no embedder required.
            // Set VECDB_SKIP_PROBE to prevent LocalEmbedder from eagerly loading the ONNX model.
            unsafe {
                std::env::set_var("VECDB_SKIP_PROBE", "true");
            }
            use std::sync::Arc;
            use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
            use vecq::detection::HybridDetector;
            let file_detector = Arc::new(HybridDetector::new());
            let parser_factory = Arc::new(VecqParserFactory);

            let services = vecdb_core::CoreServices::from_config(
                &config,
                file_detector.clone(),
                parser_factory.clone(),
            );
            let _core = vecdb_core::Core::new(&resolution, services).await?;
            commands::delete::run(args, &config).await?;
        }
        Commands::Snapshot(args) => {
            commands::snapshot::run(args, &config, profile_arg, overrides).await?
        }
        Commands::Man(args) => commands::man::run(args)?,
        Commands::Config(args) => commands::config::run(args, &mut config, profile_arg, overrides)?,
        Commands::Optimize(args) => {
            commands::optimize::run(args, &config, profile_arg, overrides).await?
        }
        Commands::History(args) => {
            commands::history::run(args, &config, profile_arg, overrides).await?
        }
        Commands::EnableUsages(args) => commands::enable_usages::run(args).await?,
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
