use clap::Args;
use colored::*;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use vecdb_core::config::Config;
use vecq::detection::HybridDetector;

use vecdb_core::parsers::vecq_adapter::VecqParserFactory;

#[derive(Args, Debug)]
pub struct DeleteArgs {
    /// Name of the collection to delete
    pub collection: Option<String>,

    /// Delete ALL collections
    #[arg(long)]
    pub all: bool,

    /// Force deletion without confirmation prompt (NOT RECOMMENDED)
    #[arg(long, alias = "yes", hide = true)]
    pub force: bool,

    /// Target specific profile to determine Qdrant endpoint
    /// Useful when same collection name exists in multiple endpoints
    #[arg(long, short = 'P')]
    pub profile: Option<String>,

    /// Target specific Qdrant URL
    /// Useful when same collection name exists in multiple endpoints
    #[arg(long)]
    pub url: Option<String>,
}

pub async fn run(args: DeleteArgs, config: &Config) -> anyhow::Result<()> {
    if !args.all && args.collection.is_none() {
        anyhow::bail!("Please specify a collection name or use --all");
    }

    if args.all && args.collection.is_some() {
        anyhow::bail!("Cannot specify both a collection name and --all");
    }

    // Delete only needs a Qdrant endpoint. It never embeds, so the embedder in
    // the resolution is constructed but unused — `--url` bypasses config
    // entirely, which is the point of the flag.
    let mut resolution = config.resolve(args.profile.as_deref(), None)?;
    if let Some(ref url) = args.url {
        resolution.qdrant_url = url.clone();
    }

    // Delete never searches and never embeds, so routing keys and path rules
    // are empty rather than inherited.
    let services = vecdb_core::CoreServices {
        smart_routing_keys: vec![],
        path_rules: vec![],
        max_concurrent_requests: 4,
        fastembed_cache_path: Some(config.fastembed_cache_path.clone()),
        allow_embed_truncation: false,
        file_detector: std::sync::Arc::new(HybridDetector::new()),
        parser_factory: std::sync::Arc::new(VecqParserFactory),
    };

    let core = vecdb_core::Core::new(&resolution, services).await?;

    let collections = if args.all {
        let cols = core.list_collections().await?;
        cols.into_iter().map(|c| c.name).collect()
    } else {
        vec![args.collection.unwrap()]
    };

    if collections.is_empty() {
        println!("No collections found to delete.");
        return Ok(());
    }

    if !args.force {
        println!("{}", "⚠️  WARNING: DESTRUCTIVE ACTION ⚠️".red().bold());
        if args.all {
            println!(
                "You are about to PERMANENTLY DELETE {} collections:",
                collections.len()
            );
            for c in &collections {
                println!(" - {}", c);
            }
        } else {
            println!(
                "You are about to PERMANENTLY DELETE collection '{}'",
                collections[0].bold()
            );
        }
        println!("This action CANNOT be undone.");
        println!();

        let token = generate_token();

        let input: String = dialoguer::Input::new()
            .with_prompt(format!(
                "To confirm, type the security token [{}]",
                token.yellow().bold()
            ))
            .interact_text()?;

        if input.trim() != token {
            println!("{}", "Confirmation failed. Deletion aborted.".red());
            return Ok(());
        }
    }

    for collection in collections {
        print!("Deleting '{}'... ", collection);
        std::io::stdout().flush()?;
        match core.delete_collection(&collection).await {
            Ok(_) => {
                println!("{}", "Done".green());
                println!(
                    "  Note: Re-ingesting will re-process files — the Qdrant collection UUID has changed."
                );
            }
            Err(e) => println!("{}", format!("Failed: {}", e).red()),
        }
    }

    Ok(())
}

fn generate_token() -> String {
    let chars = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let len = chars.len();
    let mut token = String::new();
    let start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let mut seed = start;
    for _ in 0..4 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let idx = (seed as usize) % len;
        token.push(chars.chars().nth(idx).unwrap());
    }

    token
}
