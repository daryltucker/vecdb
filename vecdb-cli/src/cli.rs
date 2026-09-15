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
    // No leading "vecdb": clap prepends the command name to whatever `version`
    // holds, so `"vecdb v1.1.1 …"` rendered as `vecdb vecdb v1.1.1 …`.
    //
    // "ONNX Runtime: {}" and NOT "ONNX v{}". `get_ort_version()` returns a
    // version number for a static build but a sentence for `cuda-dynamic`
    // ("dynamic — requires API 24 via ORT_DYLIB_PATH"), and a hardcoded "v"
    // rendered that as "ONNX vdynamic — …". The label must not assume the
    // shape of a value that legitimately has two shapes.
    // Parsed by tests/tier2_ort_distribution.py; documented in docs/GPU.md.
    let long_version = format!(
        "v{} (git:{})\nONNX Runtime: {}",
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
            // Delete only needs the backend (Qdrant) — no embedder required.
            // Set VECDB_SKIP_PROBE to prevent LocalEmbedder from eagerly loading the ONNX model.
            unsafe {
                std::env::set_var("VECDB_SKIP_PROBE", "true");
            }

            // EVERYTHING ELSE THAT WAS HERE HAS MOVED INTO `delete::run`, and
            // the move is the bug fix — not tidying. See the module comment in
            // commands/delete.rs.
            //
            // This arm used to resolve the endpoint itself, evaluate the
            // `--all` locality guard against THAT resolution, and then build a
            // `Core` it immediately threw away. `delete::run` resolved a second
            // time, from different inputs, and the deletion used the second
            // answer. A guard that inspects one endpoint while the delete hits
            // another is not a guard.
            commands::delete::run(args, &config, profile_arg, overrides).await?;
        }
        Commands::Snapshot(args) => {
            commands::snapshot::run(args, &config, profile_arg, overrides).await?
        }
        Commands::Man(args) => commands::man::run(args)?,
        Commands::Config(args) => {
            commands::config::run(args, &mut config, profile_arg, overrides, format)?
        }
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
