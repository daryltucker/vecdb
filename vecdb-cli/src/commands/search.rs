use std::sync::Arc;
use vecdb_common::output::OutputFormat;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecq::detection::HybridDetector;

pub async fn run(
    args: vecdb_core::tools::SearchArgs,
    config: &Config,
    profile_name: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
    format: OutputFormat,
) -> anyhow::Result<()> {
    let resolution = config.resolve_with(profile_name, args.collection.as_deref(), overrides)?;
    let display_profile = resolution.profile_name.clone();

    let collection = resolution.collection.as_deref()
        .ok_or_else(|| anyhow::anyhow!(
            "No collection specified. Use -c <name>, or point a collection to profile \"{}\" via `profile = \"{}\"` in config.",
            display_profile, display_profile
        ))?;
    let show_progress = format == OutputFormat::Markdown && OUTPUT.is_interactive;

    if show_progress {
        println!(
            "Using Profile: {} (Collection: {})",
            display_profile, collection
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

    // Built once so --limit and --min-score cannot diverge from the MCP path.
    let params = args.to_search_params();

    if show_progress {
        println!(
            "Searching in collection: {} for: {}",
            collection, args.query
        );
    }

    let (results, applied_filters) = if args.use_smart() {
        core.search_smart(collection, &args.query, params).await?
    } else {
        let results = core.search(collection, &args.query, params).await?;
        (results, serde_json::Map::new())
    };

    match format {
        OutputFormat::Json => {
            // Envelope rather than a bare array: a consumer that filtered its own
            // query needs to see what the filter resolved to, and needs to tell
            // "no matches" apart from "matches, but scoped away."
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "collection": collection,
                    "query": args.query,
                    "limit": args.limit.unwrap_or(vecdb_core::config::DEFAULT_SEARCH_LIMIT),
                    "min_score": args.min_score,
                    "applied_filters": applied_filters,
                    // Same field, same meaning as the MCP envelope. The two
                    // interfaces describe one response format; a caller that
                    // learns to read `result_count == limit` as "truncated"
                    // from the tool description must find it here too.
                    "result_count": results.len(),
                    "results": results,
                }))?
            );
        }
        _ => {
            if !applied_filters.is_empty() {
                let shown: Vec<String> = applied_filters
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or_default()))
                    .collect();
                println!("Filters applied: {}", shown.join(", "));
            }
            if results.is_empty() {
                // Say why the result set may be empty. "No results found" alone
                // reads as "nothing indexed" even when the search was narrowed
                // by a filter or cut off by a score threshold.
                let mut reasons = Vec::new();
                if !applied_filters.is_empty() {
                    reasons.push("the applied filters".to_string());
                }
                if let Some(min) = args.min_score {
                    reasons.push(format!("min_score {}", min));
                }
                if reasons.is_empty() {
                    println!("No results found.");
                } else {
                    println!("No results found (narrowed by {}).", reasons.join(" and "));
                }
            } else {
                for (i, result) in results.iter().enumerate() {
                    let path = result
                        .metadata
                        .get("path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let line_start = result.metadata.get("line_start").and_then(|v| v.as_u64());
                    let line_end = result.metadata.get("line_end").and_then(|v| v.as_u64());

                    let location = if let (Some(s), Some(e)) = (line_start, line_end) {
                        format!("{} [L{}-{}]", path, s, e)
                    } else {
                        path.to_string()
                    };

                    println!(
                        "\n--- Result {} (Score: {:.4}) | {} ---",
                        i + 1,
                        result.score,
                        location
                    );
                    println!("{}", result.content.trim());
                }
            }
        }
    }

    Ok(())
}
