/*
 * PURPOSE:
 *   Implements the `status` command.
 *   Displays current configuration, connectivity, and collection stats.
 */

use clap::Args;
use std::sync::Arc;
use termimad::{crossterm::style::Color, MadSkin};
use vecdb_core::config::Config;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Show details for a specific Job ID
    #[arg(short, long)]
    pub id: Option<String>,
}

pub async fn run(
    args: StatusArgs,
    config: &Config,
    profile_name_opt: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
    format: vecdb_common::output::OutputFormat,
) -> anyhow::Result<()> {
    let resolution = config.resolve_with(profile_name_opt, None, overrides)?;
    let profile_name = resolution.profile_name.clone();

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let services = vecdb_core::CoreServices::from_config(
        config,
        file_detector.clone(),
        parser_factory.clone(),
    );
    let core_result = vecdb_core::Core::new(&resolution, services).await;

    let job_registry = vecdb_core::jobs::JobRegistry::new().ok();
    let local_jobs = job_registry
        .as_ref()
        .and_then(|r| r.load().ok())
        .unwrap_or_default();

    if matches!(format, vecdb_common::output::OutputFormat::Json) {
        use serde_json::json;
        let mut status = json!({
            "profile": profile_name,
            "qdrant_url": resolution.qdrant_url,
            "embedder": {
                "type": resolution.backend.kind.to_string(),
                "model": resolution.embedder.model,
                "name": resolution.embedder_name,
                "backend": resolution.backend_name
            },
            "ollama_url": if resolution.is_ollama() { Some(resolution.ollama_url()) } else { None },
            "connectivity": {
                "qdrant": false,
                "error": serde_json::Value::Null
            },
            "collections": [],
            "background_tasks": [],
            "local_jobs": local_jobs
        });

        match core_result {
            Ok(ref core) => {
                status["connectivity"]["qdrant"] = json!(true);
                match core.list_collections().await {
                    Ok(collections) => {
                        let cols_json: Vec<serde_json::Value> = collections
                            .into_iter()
                            .map(|c| {
                                json!({
                                    "name": c.name,
                                    "vector_count": c.vector_count,
                                    "vector_size": c.vector_size,
                                    "is_active": resolution.collection.as_deref() == Some(c.name.as_str())
                                })
                            })
                            .collect();
                        status["collections"] = json!(cols_json);
                    }
                    Err(e) => {
                        status["connectivity"]["collections_error"] = json!(e.to_string());
                    }
                }

                match core.list_tasks().await {
                    Ok(tasks) => status["background_tasks"] = json!(tasks),
                    // Reported, not omitted. A caller cannot tell an absent key
                    // from an empty one, and "unavailable" is not "none".
                    Err(e) => status["background_tasks_error"] = json!(e.to_string()),
                }
            }
            Err(e) => {
                status["connectivity"]["error"] = json!(e.to_string());
            }
        }

        if let Some(target_id) = &args.id {
            if let Some(job) = local_jobs.iter().find(|j| &j.id == target_id) {
                println!("{}", serde_json::to_string_pretty(&job)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        return Ok(());
    }

    // Detail View
    if let Some(target_id) = &args.id {
        let skin = make_custom_skin();
        if let Some(job) = local_jobs.iter().find(|j| &j.id == target_id) {
            skin.print_text(&format!(" # Job Details: `{}`", target_id));
            skin.print_text(&format!("* **Type**: `{}`", job.job_type));
            skin.print_text(&format!("* **Collection**: `{}`", job.collection));
            skin.print_text(&format!("* **Status**: `{:?}`", job.status));
            skin.print_text(&format!("* **Progress**: `{:.1}%`", job.progress * 100.0));
            skin.print_text(&format!("* **PID**: `{}`", job.pid));
            skin.print_text(&format!("* **Started**: `{}`", job.started_at));
            skin.print_text(&format!("* **Updated**: `{}`", job.updated_at));
        } else {
            skin.print_text(&format!(" # Job ID `{}` not found.", target_id));
        }
        return Ok(());
    }

    // Interactive Mode (Original)
    let skin = make_custom_skin();

    println!();
    skin.print_text(" # System Status");
    println!();

    // Configuration Table
    skin.print_text(&format!("* **Profile**: `{}`", profile_name));
    skin.print_text(&format!("* **Qdrant URL**: `{}`", resolution.qdrant_url));
    skin.print_text(&format!(
        "* **Embedder**: `{}` — {} on backend `{}` ({})",
        resolution.embedder_name,
        resolution.embedder.model,
        resolution.backend_name,
        resolution.backend.kind
    ));
    if resolution.is_ollama() {
        skin.print_text(&format!("* **Ollama URL**: `{}`", resolution.ollama_url()));
    }

    // Connectivity Check
    skin.print_text("\n## Connectivity");

    match core_result {
        Ok(core) => {
            skin.print_text("* **Qdrant**: **ONLINE** (Connected)");

            // Background Tasks (from Backend)
            skin.print_text("\n## Active Remote Tasks (Qdrant)");
            let task_result = core.list_tasks().await;
            if let Err(ref e) = task_result {
                skin.print_text(&format!(" *Unavailable: {e}*"));
            }
            if let Ok(tasks) = task_result {
                if tasks.is_empty() {
                    skin.print_text(" *No active remote tasks.*");
                } else {
                    println!("{:<10} | {:<20} | {:<20}", "ID", "Type", "Status");
                    println!("{:-<10}-+-{:-<20}-+-{:-<20}", "", "", "");
                    for t in tasks {
                        println!("{:<10} | {:<20} | {:<20}", t.id, t.description, t.status);
                    }
                }
            }

            // Local Jobs
            skin.print_text("\n## Active Local Jobs (Ingestion)");
            if local_jobs.is_empty() {
                skin.print_text(" *No active local jobs.*");
            } else {
                println!(
                    "{:<10} | {:<10} | {:<15} | {:<10} | {:<10}",
                    "ID", "Type", "Collection", "Progress", "PID"
                );
                println!(
                    "{:-<10}-+-{:-<10}-+-{:-<15}-+-{:-<10}-+-{:-<10}",
                    "", "", "", "", ""
                );
                for j in local_jobs {
                    println!(
                        "{:<10} | {:<10} | {:<15} | {:<10.1}% | {:<10}",
                        j.id,
                        j.job_type,
                        j.collection,
                        j.progress * 100.0,
                        j.pid
                    );
                }
            }

            // Collection Overview
            skin.print_text("\n## Collections");
            match core.list_collections().await {
                Ok(collections) => {
                    if collections.is_empty() {
                        skin.print_text(" *No collections found.*");
                    } else {
                        println!(
                            "{:<15} | {:<10} | {:<10} | {:<10}",
                            "Name", "Vectors", "Size", "Active"
                        );
                        println!("{:-<15}-+-{:-<10}-+-{:-<10}-+-{:-<10}", "", "", "", "");
                        for c in collections {
                            let active_str =
                                if resolution.collection.as_deref() == Some(c.name.as_str()) {
                                    "YES"
                                } else {
                                    ""
                                };
                            println!(
                                "{:<15} | {:<10} | {:<10} | {:<10}",
                                c.name,
                                c.vector_count.unwrap_or(0),
                                c.vector_size.unwrap_or(0),
                                active_str
                            );
                        }
                    }
                }
                Err(e) => {
                    skin.print_text(&format!("* **Collections Error**: `{}`", e));
                }
            }
        }
        Err(e) => {
            skin.print_text(&format!("* **Qdrant**: **OFFLINE** ({})", e));
        }
    }

    // ONNX Runtime Status
    skin.print_text("\n## ONNX Runtime (Accelerators)");
    let ort_version = vecdb_core::get_ort_version();
    let providers = vecdb_core::get_ort_providers();

    skin.print_text(&format!("* **Version**: `{}`", ort_version));

    let mut providers_formatted = vec![];
    for p in providers {
        if p.contains("CUDA") {
            // Highlight CUDA in Green/Bold
            providers_formatted.push(format!("**{}**", p));
        } else {
            providers_formatted.push(p);
        }
    }

    skin.print_text(&format!(
        "* **Providers**: [{}]",
        providers_formatted.join(", ")
    ));
    if !providers_formatted.iter().any(|p| p.contains("CUDA")) && resolution.use_gpu.value {
        skin.print_text("\n> [!WARNING]\n> CUDA provider missing but GPU requested!");
        skin.print_text("> See `docs/vecq/GPU.md` to install `libonnxruntime_providers_cuda.so`");
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
