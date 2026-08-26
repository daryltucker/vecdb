use clap::Args;
use std::sync::Arc;
use vecdb_core::config::{Config, QuantizationType};
use vecdb_core::output::OUTPUT;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct OptimizeArgs {
    /// Collection to optimize
    #[arg(index = 1)]
    pub collection: String,
}

pub async fn run(
    args: OptimizeArgs,
    config: &Config,
    profile_name: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    let resolution = config.resolve_with(profile_name, Some(&args.collection), overrides)?;
    let q_type = resolution
        .quantization
        .clone()
        .unwrap_or(QuantizationType::Scalar);

    if OUTPUT.is_interactive {
        println!(
            "Optimizing collection '{}' with strategy: {:?}",
            args.collection, q_type
        );
    }

    let file_detector = Arc::new(HybridDetector::new());
    let parser_factory = Arc::new(VecqParserFactory);

    let services = vecdb_core::CoreServices::from_config(
        config,
        file_detector.clone(),
        parser_factory.clone(),
    );
    let core = vecdb_core::Core::new(&resolution, services).await?;

    core.optimize_collection(&args.collection, q_type).await?;
    println!("Optimization triggered. Check Qdrant logs for background progress.");
    Ok(())
}
