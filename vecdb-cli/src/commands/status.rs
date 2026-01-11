/*
 * PURPOSE:
 *   Implements the `status` command.
 *   Displays current configuration, connectivity, and collection stats.
 *
 * AESTHETICS:
 *   "Sexy/Sleek Hacker" - ANSI colors, clear tables, status indicators.
 */

use clap::Args;
use termimad::{crossterm::style::Color, MadSkin};
use vecdb_core::config::Config;
use std::sync::Arc;
use crate::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

pub async fn run(args: StatusArgs, config: &Config, profile_name: &str) -> anyhow::Result<()> {
    // 1. Resolve Profile
    let profile = config.get_profile(Some(profile_name))?;

    // Prepare shared services
    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    // 2. Connectivity Check & Core Init
    let core_result = vecdb_core::Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &profile.embedding_model,
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(None),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        file_detector.clone(),
        parser_factory.clone(),
    ).await;

    if args.json {
        use serde_json::json;
        let mut status = json!({
            "profile": profile_name,
            "qdrant_url": profile.qdrant_url,
            "embedder": {
                "type": profile.embedder_type,
                "model": profile.embedding_model
            },
            "ollama_url": if profile.embedder_type == "ollama" { Some(&profile.ollama_url) } else { None },
            "connectivity": {
                "qdrant": false,
                "error": serde_json::Value::Null
            },
            "collections": []
        });

        match core_result {
            Ok(core) => {
                status["connectivity"]["qdrant"] = json!(true);
                match core.list_collections().await {
                    Ok(collections) => {
                        let cols_json: Vec<serde_json::Value> = collections.into_iter().map(|c| json!({
                            "name": c.name,
                            "vector_count": c.vector_count,
                            "vector_size": c.vector_size,
                            "is_active": c.name == profile.default_collection_name
                        })).collect();
                        status["collections"] = json!(cols_json);
                    },
                    Err(e) => {
                         status["connectivity"]["collections_error"] = json!(e.to_string());
                    }
                }
            },
            Err(e) => {
                 status["connectivity"]["error"] = json!(e.to_string());
            }
        }
        
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

    // Interactive Mode (Original)
    let skin = make_custom_skin();
    
    println!(); 
    skin.print_text(" # System Status");
    println!();

    // Configuration Table
    skin.print_text(&format!("* **Profile**: `{}`", profile_name));
    skin.print_text(&format!("* **Qdrant URL**: `{}`", profile.qdrant_url));
    skin.print_text(&format!("* **Embedder**: `{}` ({})", profile.embedder_type, profile.embedding_model));
    if profile.embedder_type == "ollama" {
        skin.print_text(&format!("* **Ollama URL**: `{}`", profile.ollama_url));
    }
    
    // Connectivity Check
    skin.print_text("\n## Connectivity");
    
    match core_result {
        Ok(core) => {
             skin.print_text("* **Qdrant**: **ONLINE** (Connected)");
             
             // Collection Stats
             skin.print_text("\n## Collections");
             match core.list_collections().await {
                 Ok(collections) => {
                     if collections.is_empty() {
                         skin.print_text(" *No collections found.*");
                     } else {
                         // Table Header
                         println!("{:<20} | {:<15} | {:<10}", "Name", "Vectors", "Dim");
                         println!("{:-<20}-+-{:-<15}-+-{:-<10}", "", "", "");
                         
                         for c in collections {
                             let count_str = c.vector_count.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
                             let dim_str = c.vector_size.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
                             
                             // Highlight the active collection from profile
                             if c.name == profile.default_collection_name {
                                 skin.print_text(&format!("**{:<20}** | {:<15} | {:<10} *(Active)*", c.name, count_str, dim_str));
                             } else {
                                 println!("{:<20} | {:<15} | {:<10}", c.name, count_str, dim_str);
                             }
                         }
                     }
                 }
                 Err(e) => {
                     skin.print_text(&format!("* **Error fetching collections**: {}", e));
                 }
             }
        }
        Err(e) => {
            skin.print_text(&format!("* **Qdrant**: **OFFLINE** (Error: {})", e));
            skin.print_text("\n> [!WARNING]\n> Cannot connect to backend. Ensure Qdrant is running.");
        }
    }
    
    println!();
    Ok(())
}

fn make_custom_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.headers[0].set_fg(Color::Cyan);
    skin.headers[1].set_fg(Color::Magenta); 
    skin.bold.set_fg(Color::Green);
    skin.italic.set_fg(Color::DarkGrey);
    skin
}
