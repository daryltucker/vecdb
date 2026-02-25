use crate::vecq_adapter::VecqParserFactory;
use clap::Args;
use std::sync::Arc;
use vecdb_core::config::{Config, QuantizationType};
use vecdb_core::output::OUTPUT;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct OptimizeArgs {
    /// Collection to optimize
    #[arg(index = 1)]
    pub collection: String,
}

pub async fn run(args: OptimizeArgs, config: &Config, profile_name: &str) -> anyhow::Result<()> {
    let profile = config.resolve_profile(Some(profile_name), Some(&args.collection))?;
    let q_type = profile.quantization.clone().unwrap_or(QuantizationType::Scalar);

    if OUTPUT.is_interactive {
        println!(
            "Optimizing collection '{}' with strategy: {:?}",
            args.collection, q_type
        );
    }

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let core = vecdb_core::Core::new(
        &profile.qdrant_url,
        &profile.ollama_url,
        &config.resolve_embedding_model(&profile),
        profile.accept_invalid_certs,
        &profile.embedder_type,
        Some(config.fastembed_cache_path.clone()),
        config.resolve_local_use_gpu(Some(&args.collection)),
        profile.qdrant_api_key.clone(),
        profile.ollama_api_key.clone(),
        config.smart_routing_keys.clone(),
        config.ingestion.path_rules.clone(),
        config.ingestion.max_concurrent_requests,
        config.resolve_gpu_batch_size(&profile, Some(args.collection.as_str())),
        profile.num_ctx,
        file_detector.clone(),
        parser_factory.clone(),
    )
    .await?;

    core.optimize_collection(&args.collection, q_type).await?;
    println!("Optimization triggered. Check Qdrant logs for background progress.");
    Ok(())
}
