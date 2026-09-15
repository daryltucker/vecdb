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

/// Delete collections.
///
/// THE ENDPOINT IS RESOLVED EXACTLY ONCE, HERE, AND THE `--all` GUARD IS
/// EVALUATED AGAINST THAT SAME ANSWER.
///
/// It used to be resolved twice. `cli.rs` resolved from the GLOBAL `--profile`
/// and checked the locality guard against that; this function then re-resolved
/// from the SUBCOMMAND-LOCAL `-P`, applied `--url` afterwards, and deleted
/// against the second answer.
///
/// `--url` was the exploitable half — no trickery, just flags working as
/// documented:
///
///     vecdb delete --all --url http://<remote>:PORT
///
/// `--url` is applied HERE, after cli.rs had already run the guard, so the
/// guard never saw it. It read the default profile's localhost, passed, and the
/// command then listed and deleted every collection on the remote host.
///
/// The `-P` half was NOT exploitable, though the audit listed it as such:
/// `--profile` on the root is `global = true`, so `-P` populates `cli.profile`
/// and `args.profile` together and they cannot disagree. Measured against the
/// pre-fix binary, that route was correctly refused.
///
/// Keeping one resolution makes the whole class unrepresentable rather than
/// patching the one hole: there is no second answer left to disagree with.
///
/// `profile_arg` is the global `--profile`. The subcommand's own `-P` wins when
/// present — it exists precisely to aim a delete at one endpoint — but when it
/// is absent the global flag is now honoured. Previously delete ignored the
/// global `--profile` silently, so `vecdb --profile remote delete foo` operated
/// on the default profile and reported success.
pub async fn run(
    args: DeleteArgs,
    config: &Config,
    profile_arg: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    if !args.all && args.collection.is_none() {
        anyhow::bail!("Please specify a collection name or use --all");
    }

    if args.all && args.collection.is_some() {
        anyhow::bail!("Cannot specify both a collection name and --all");
    }

    // Delete only needs a Qdrant endpoint. It never embeds, so the embedder in
    // the resolution is constructed but unused — `--url` bypasses config
    // entirely, which is the point of the flag.
    //
    // The COLLECTION must be passed to `resolve`. It used to pass `None`, which
    // resolved the default profile's store and ignored `[collections.<name>]`
    // entirely — so `vecdb delete notes-archive`, a collection routed to a remote
    // store, deleted a non-existent collection from the LOCAL one and printed
    // "Done". Deleting a name that is not there is not an error in Qdrant, so
    // the no-op looked exactly like a success while the real data survived.
    let profile = args.profile.as_deref().or(profile_arg);
    let mut resolution = config.resolve_with(profile, args.collection.as_deref(), overrides)?;
    if let Some(ref url) = args.url {
        resolution.qdrant_url = url.clone();
    }

    // The locality guard, evaluated on the FINAL endpoint — after the profile
    // choice and after `--url`. Nothing may change `resolution.qdrant_url`
    // below this point; if that ever becomes necessary, this check moves with
    // it rather than being left behind.
    //
    // NOTE ON WHAT THIS DOES *NOT* PROTECT. It is a substring test that blocks
    // only REMOTE bulk deletion. Production here is `localhost:6333`, so
    // `vecdb delete --all` against local collections is fully permitted and is
    // stopped by nothing but the interactive token prompt below. Making local
    // data safe needs a protected-collection denylist, which is designed
    // (`[protected]`) but NOT implemented — do not read this guard as one.
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

    // Delete never searches and never embeds, so routing keys and path rules
    // are empty rather than inherited.
    let services = vecdb_core::CoreServices {
        smart_routing_keys: vec![],
        fastembed_cache_path: Some(config.fastembed_cache_path.clone()),
        allow_embed_truncation: false,
        file_detector: std::sync::Arc::new(HybridDetector::new()),
        parser_factory: std::sync::Arc::new(VecqParserFactory::default()),
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

    // Failures are COUNTED, not just printed. Every branch below used to
    // `continue` or swallow, and the function returned `Ok(())` regardless — so
    // `vecdb delete X && echo gone` printed "gone" after a store that could not
    // be reached and a collection that was never touched. Printing red text is
    // not a failure signal to anything that is not a human reading a terminal,
    // and the agent interface is the primary consumer here.
    //
    // A collection that is genuinely ABSENT is not counted: the requested end
    // state ("this collection does not exist") already holds, and delete is
    // meant to be idempotent. Being unable to ASK is the failure.
    let mut failures: Vec<String> = Vec::new();

    for collection in collections {
        print!("Deleting '{}' at {}... ", collection, resolution.qdrant_url);
        std::io::stdout().flush()?;

        // Check first. Qdrant treats deleting an absent collection as success,
        // so without this a wrong endpoint, a typo, or an already-deleted name
        // all report "Done" — and the operator believes data is gone when it is
        // not. Say which endpoint was checked, so a surprise is diagnosable.
        //
        // `Err` here means the store could not be asked. It must stay distinct
        // from `Ok(false)`: the backend used to flatten every transport error
        // into `false`, which made this branch unreachable and turned an
        // unreachable host into "not found". See backends/qdrant.rs.
        match core.collection_exists(&collection).await {
            Ok(false) => {
                println!("{}", "not found — nothing deleted".yellow());
                continue;
            }
            Err(e) => {
                println!("{}", format!("Failed: could not reach store: {e}").red());
                failures.push(format!("{collection}: could not reach store: {e}"));
                continue;
            }
            Ok(true) => {}
        }

        match core.delete_collection(&collection).await {
            Ok(_) => {
                println!("{}", "Done".green());
                println!(
                    "  Note: Re-ingesting will re-process files — the Qdrant collection UUID has changed."
                );
            }
            Err(e) => {
                println!("{}", format!("Failed: {}", e).red());
                failures.push(format!("{collection}: {e}"));
            }
        }
    }

    if !failures.is_empty() {
        anyhow::bail!(
            "{} of the requested deletions did not happen at {}:\n  {}",
            failures.len(),
            resolution.qdrant_url,
            failures.join("\n  ")
        );
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
