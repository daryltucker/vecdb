use clap::Args;
use colored::*;
use std::io::Write;
use vecdb_core::Core;
use std::time::{SystemTime, UNIX_EPOCH};

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
}

pub async fn run(core: &Core, args: DeleteArgs) -> anyhow::Result<()> {
    if !args.all && args.collection.is_none() {
        anyhow::bail!("Please specify a collection name or use --all");
    }

    if args.all && args.collection.is_some() {
        anyhow::bail!("Cannot specify both a collection name and --all");
    }

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
            println!("You are about to PERMANENTLY DELETE {} collections:", collections.len());
            for c in &collections {
                println!(" - {}", c);
            }
        } else {
            println!("You are about to PERMANENTLY DELETE collection '{}'", collections[0].bold());
        }
        println!("This action CANNOT be undone.");
        println!();

        // Generate simple 4-char token
        let token = generate_token();
        
        let input: String = dialoguer::Input::new()
            .with_prompt(format!("To confirm, type the security token [{}]", token.yellow().bold()))
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
            Ok(_) => println!("{}", "Done".green()),
            Err(e) => println!("{}", format!("Failed: {}", e).red()),
        }
    }

    Ok(())
}

fn generate_token() -> String {
    let chars = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let len = chars.len();
    let mut token = String::new();
    let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    
    // Simple PRNG based on time
    let mut seed = start;
    for _ in 0..4 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let idx = (seed as usize) % len;
        token.push(chars.chars().nth(idx).unwrap());
    }
    
    token
}
