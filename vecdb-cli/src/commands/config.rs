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

pub fn run(args: ConfigArgs, config: &mut Config) -> anyhow::Result<()> {
    match args.command {
        ConfigCommands::SetQuantization { collection, r#type } => {
            let q_type: QuantizationType = r#type.into();

            let c_config = config.collections.entry(collection.clone()).or_insert(
                vecdb_core::config::CollectionConfig {
                    name: collection.clone(),
                    description: None,
                    embedder_type: None,
                    embedding_model: None,
                    ollama_url: None,
                    chunk_size: None,
                    chunk_overlap: None,
                    max_chunk_size: None,
                    use_gpu: None,
                    qdrant_api_key: None,
                    ollama_api_key: None,
                    profile: None,
                    num_ctx: None,
                    gpu_batch_size: None,
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
