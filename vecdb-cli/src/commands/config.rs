use crate::QuantizationArg;
use clap::{Args, Subcommand};
use vecdb_core::config::{Config, QuantizationType};

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Show the effective settings and where each one came from.
    ///
    /// Answers "what will actually happen if I ingest right now", which is not
    /// the same question as "what is in my config file" — most values arrive by
    /// falling through several layers.
    Show {
        /// Resolve as if ingesting into this collection
        #[arg(long, short)]
        collection: Option<String>,
    },
    /// Set quantization for a collection
    SetQuantization {
        /// Collection name
        #[arg(index = 1)]
        collection: String,
        /// Quantization type (scalar, binary, none)
        #[arg(value_enum, index = 2)]
        r#type: QuantizationArg,
    },
}

pub fn run(
    args: ConfigArgs,
    config: &mut Config,
    profile_name: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    match args.command {
        ConfigCommands::Show { collection } => {
            return show_resolved(config, profile_name, collection.as_deref(), overrides);
        }
        ConfigCommands::SetQuantization { collection, r#type } => {
            let q_type: QuantizationType = r#type.into();

            let c_config = config.collections.entry(collection.clone()).or_insert(
                vecdb_core::config::CollectionConfig {
                    name: collection.clone(),
                    description: None,
                    profile: None,
                    embedder: None,
                    qdrant_url: None,
                    qdrant_api_key: None,
                    target_chunk_size: None,
                    chunk_overlap: None,
                    max_chunk_bytes: None,
                    quantization: None,
                },
            );

            c_config.quantization = Some(q_type.clone());
            config.save()?;
            println!(
                "Updated quantization for collection '{}' to {:?}",
                collection, q_type
            );
        }
    }
    Ok(())
}

/// Print every effective setting with the layer that supplied it.
///
/// Reads through `Config::resolve` — the same call the ingest path makes — rather
/// than re-walking the precedence rules for display. A viewer with its own copy
/// of the precedence logic drifts from the resolver, and then the tool that tells
/// you what will happen disagrees with what happens.
fn show_resolved(
    config: &Config,
    profile_name: Option<&str>,
    collection: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    use vecdb_core::config::Source;

    let r = config.resolve_with(profile_name, collection, overrides)?;

    println!("profile    {}", r.profile_name);
    println!("collection {}", r.collection.as_deref().unwrap_or("(none)"));
    println!("qdrant     {}", r.qdrant_url);
    println!();
    println!(
        "embedder   {}  ({} on backend {} — {})",
        r.embedder_name, r.embedder.model, r.backend_name, r.backend.kind
    );
    // Only worth a line when a flag moved it; otherwise it just repeats the
    // profile name already printed above.
    if let Source::Cli(flag) = r.embedder_source {
        println!("           embedder selected by {flag}");
    }
    if let Source::Cli(flag) = r.backend_source {
        println!("           backend  selected by {flag} (model and tuning unchanged)");
    }
    if r.is_ollama() {
        println!("           {}", r.ollama_url());
    }
    println!();

    let row = |key: &str, value: String, unit: &str, source: &Source| {
        println!("  {key:<16} {value:>10} {unit:<8} <- {source}");
    };

    row(
        "target_chunk_size",
        r.target_chunk_size.value.to_string(),
        "tokens",
        &r.target_chunk_size.source,
    );
    row(
        "chunk_overlap",
        r.chunk_overlap.value.to_string(),
        "tokens",
        &r.chunk_overlap.source,
    );
    row(
        "max_chunk_bytes",
        r.max_chunk_bytes.value.to_string(),
        "bytes",
        &r.max_chunk_bytes.source,
    );
    row(
        "on_oversize",
        r.on_oversize.value.to_string(),
        "",
        &r.on_oversize.source,
    );

    // Only the knobs that apply to this backend. `num_ctx` on a fastembed
    // embedder, or `use_gpu` on a remote one, would be noise pretending to be
    // configuration.
    if r.is_ollama() {
        row(
            "num_ctx",
            r.num_ctx.value.to_string(),
            "tokens",
            &r.num_ctx.source,
        );
        row(
            "batch_inputs",
            r.batch.value.to_string(),
            "inputs",
            &r.batch.source,
        );
    } else {
        row(
            "batch_rows",
            r.batch.value.to_string(),
            "rows",
            &r.batch.source,
        );
        row(
            "use_gpu",
            r.use_gpu.value.to_string(),
            "",
            &r.use_gpu.source,
        );
    }

    if let Some(dim) = r.embedder.dimension {
        println!(
            "  {:<16} {:>10} {:<8} <- {}",
            "dimension",
            dim,
            "dims",
            Source::Embedder(r.embedder_name.clone())
        );
        println!("\n  note: dimension is a Matryoshka truncation target. Once a collection");
        println!("        is written at it, the genesis record pins it — changing it later");
        println!("        means a full re-ingest.");
    }

    println!();
    println!(
        "  note: target_chunk_size counts tokens (tokenizer = {}); max_chunk_bytes counts bytes.",
        config.ingestion.tokenizer
    );

    if r.is_ollama() {
        match vecdb_core::config::check_chunk_fit(r.target_chunk_size.value, r.num_ctx.value) {
            vecdb_core::config::ChunkFit::Impossible => println!(
                "  ERROR: target_chunk_size {} does not fit num_ctx {} — every full-size chunk \
                 would trip on_oversize.",
                r.target_chunk_size.value, r.num_ctx.value
            ),
            vecdb_core::config::ChunkFit::Tight => println!(
                "  WARNING: target_chunk_size {} is within {:.0}% of num_ctx {} — some chunks \
                 will likely trip on_oversize.",
                r.target_chunk_size.value,
                vecdb_core::config::TOKENIZER_MARGIN * 100.0,
                r.num_ctx.value
            ),
            vecdb_core::config::ChunkFit::Ok => {}
        }
    }

    Ok(())
}
