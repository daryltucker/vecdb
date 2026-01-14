use clap::Args;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;

#[derive(Args, Debug)]
pub struct SnapshotArgs {
    #[arg(short, long)]
    pub create: bool,

    #[arg(short, long)]
    pub list: bool,

    #[arg(short, long)]
    pub download: Option<String>, // Snapshot name

    #[arg(long)]
    pub restore: Option<String>, // File path

    #[arg(short = 'C', long)]
    pub collection: Option<String>, // Optional override
}

pub async fn run(args: SnapshotArgs, config: &Config, profile_name: &str) -> anyhow::Result<()> {
    let profile = config.resolve_profile(Some(profile_name), args.collection.as_deref())?;
    let collection_name = args.collection.as_deref().unwrap_or(&profile.default_collection_name);
    
    let manager = vecdb_core::snapshot::SnapshotManager::new(&profile.qdrant_url)?;

    if args.create {
        if OUTPUT.is_interactive { println!("Creating snapshot for collection '{}'...", collection_name); }
        let name = manager.create(collection_name).await?;
        println!("Snapshot created: {}", name);
    } else if args.list {
        let snapshots = manager.list(collection_name).await?;
        if snapshots.is_empty() {
                println!("No snapshots found for collection '{}'.", collection_name);
        } else {
            println!("Snapshots for '{}':", collection_name);
            for s in snapshots {
                println!("- {}", s);
            }
        }
    } else if let Some(snap_name) = args.download {
        let output_path = std::path::Path::new(&snap_name);
        if OUTPUT.is_interactive { println!("Downloading snapshot '{}'...", snap_name); }
        manager.download(collection_name, &snap_name, output_path).await?;
        println!("Downloaded to: {:?}", output_path);
    } else if let Some(file_path) = args.restore {
            if OUTPUT.is_interactive { println!("Restoring snapshot from {:?} to collection '{}'...", file_path, collection_name); }
        manager.restore(collection_name, std::path::Path::new(&file_path)).await?;
        println!("Snapshot restored successfully.");
    } else {
        println!("Please specify an action: --create, --list, --download <NAME>, or --restore <PATH>");
    }
    
    Ok(())
}
