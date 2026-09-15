use clap::Args;
use std::path::PathBuf;
use std::sync::Arc;
use vecdb_core::config::Config;
use vecdb_core::output::OUTPUT;
use vecdb_core::parsers::vecq_adapter::VecqParserFactory;
use vecdb_core::vecdbrc::VecdbRc;
use vecq::detection::HybridDetector;

#[derive(Args, Debug)]
pub struct IngestArgs {
    /// Path(s) to files or directories to ingest. Use `-` for stdin.
    /// Glob expansion happens in your shell (bash/zsh) — vecdb receives
    /// the expanded paths. You can pass multiple paths: `ingest a.md b.md`
    #[arg(default_value = ".", num_args = 1..)]
    pub paths: Vec<PathBuf>,

    /// Collection to ingest into (created if missing)
    #[arg(long, short)]
    pub collection: Option<String>,

    /// Permit writing into a collection whose model matches on architecture and
    /// parameter size but was built at a different quantization (e.g. embedding
    /// with Q8_0 into a collection created with Q4_K_M).
    ///
    /// Off by default. Writes are strict because a bad one contaminates the
    /// collection permanently and compounds with every later ingest, whereas the
    /// equivalent risk on a read is one slightly worse ranking. Mixing builds
    /// inside one collection should be a decision, not an accident.
    #[arg(long)]
    pub allow_quantization_delta: bool,

    /// Additional metadata in key=value format (can be specified multiple times)
    #[arg(long, short = 'm')]
    pub metadata: Vec<String>,

    /// Respect .gitignore files (skips ignored files)
    #[arg(long, default_value_t = false)]
    pub respect_gitignore: bool,

    /// Ignore .vectorignore files (skip reading .vectorignore during file walking)
    #[arg(long, default_value_t = false)]
    pub ignore_vectorignore: bool,

    /// Target chunk size (tokens for text, chars for default). Overrides config.
    #[arg(long)]
    pub target_chunk_size: Option<usize>,

    /// Chunk overlap (tokens for text, chars for default). Overrides config.
    #[arg(long, short = 'o')]
    pub overlap: Option<usize>,

    /// Extension whitelist (e.g. "rs", "md")
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub extensions: Option<Vec<String>>,

    /// Exclude glob patterns (e.g. "*.tmp", "target/")
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    pub excludes: Option<Vec<String>>,

    /// Dry run: List files that would be ingested without processing
    #[arg(long)]
    pub dry_run: bool,

    /// Max concurrent file processing tasks
    #[arg(long, short = 'P')]
    pub concurrency: Option<usize>,

    /// Max concurrent GPU embedding tasks (batch size)
    #[arg(long, short = 'G')]
    pub gpu_concurrency: Option<usize>,
}

pub async fn run(
    args: IngestArgs,
    config: &Config,
    profile_name: Option<&str>,
    overrides: vecdb_core::config::Overrides<'_>,
) -> anyhow::Result<()> {
    // `.vecdbrc` is discovered BEFORE resolution, because its `[default]
    // collection` is one of the answers to "which collection?".
    //
    // It used to be read ~130 lines below the check that requires a collection,
    // so a project whose `.vecdbrc` said `collection = "code"` still died with
    // "No collection specified" — the file that answered the question was never
    // opened. Discovery walks up from the first path, so it finds the same file
    // the routing pass below will.
    let rc_result = args
        .paths
        .iter()
        .find(|p| p.to_str() != Some("-"))
        .map(|p| VecdbRc::discover(p))
        .transpose()?
        .flatten();

    // Precedence: -c wins, then `.vecdbrc [default]`, then the profile's own
    // default collection. An explicit flag is never overridden by a file.
    let rc_default_collection = rc_result
        .as_ref()
        .and_then(|(_, rc)| rc.default.as_ref())
        .and_then(|d| d.collection.as_deref());
    let requested_collection = args.collection.as_deref().or(rc_default_collection);

    // One resolution for the whole run: backend, embedder, store and chunking,
    // every layer already collapsed. `vecdb config show` prints this same thing.
    let resolution = config.resolve_with(profile_name, requested_collection, overrides)?;
    let display_profile = resolution.profile_name.clone();
    let display_profile = display_profile.as_str();

    let collection = resolution.collection.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "No collection specified. Use -c <name>, point a collection to profile \"{}\" via \
             `profile = \"{}\"` in config, or set `[default] collection` in a .vecdbrc.",
            display_profile,
            display_profile
        )
    })?;

    if OUTPUT.is_interactive && !args.dry_run {
        println!(
            "Using Profile: {} (Collection: {})",
            display_profile, collection
        );
    }

    // Check for stdin pipe: any path value of "-" means read from stdin.
    // When multiple paths are given, only one "-" is expected alone.
    if args.paths.iter().any(|p| p.to_str() == Some("-")) {
        return run_stdin(args, config, display_profile, &resolution, collection).await;
    }

    // Checked before the embedder is constructed, deliberately.
    //
    // This is a question about two numbers in a config file; it needs no network
    // and no model. Running it after `Core::new` meant an unreachable Ollama
    // masked a misconfiguration that could have been reported instantly.
    //
    // A chunk target that cannot fit the model's context window is not an edge
    // case to discover at embed time — it fires on essentially every full-size
    // chunk, forty minutes in, when the run has already cost something. The
    // numbers are known now, so say so now.
    //
    // Note what this does NOT do: it never adjusts `num_ctx` or `target_chunk_size`. A
    // value the operator wrote is used exactly as written.
    // Where the window comes from differs by backend; the checks do not.
    //
    // Both paths need checking, and the fastembed one more so: Ollama refuses an
    // over-long input, while fastembed silently truncates it to a prefix. The
    // quiet failure is the dangerous one.
    // Where the window comes from differs by backend; `Resolution` knows, and
    // `history ingest` and the MCP tool need the same answer.
    let context_tokens = resolution.context_tokens();

    if let Some(num_ctx) = context_tokens {
        let target = args
            .target_chunk_size
            .unwrap_or(resolution.target_chunk_size.value);
        match vecdb_core::config::check_chunk_fit(target, num_ctx) {
            vecdb_core::config::ChunkFit::Impossible => {
                anyhow::bail!(
                    "target_chunk_size ({target}) does not fit num_ctx ({num_ctx}) for profile \"{}\".\n\
                     \n\
                     \x20 Every full-size chunk would exceed the model's context window, so the\n\
                     \x20 on_oversize policy would fire on all of them.\n\
                     \n\
                     \x20 fix: lower target_chunk_size below {num_ctx}, or raise num_ctx if the model\n\
                     \x20      supports it. `vecdb config show -c <collection>` shows where each\n\
                     \x20      value is coming from.",
                    display_profile
                );
            }
            // Not gated on `is_interactive`. A warning only piped callers cannot
            // see is the same defect as the smart-routing filter that was
            // invisible outside a terminal — and agents are exactly who needs to
            // know the corpus may come back thinner than the source.
            vecdb_core::config::ChunkFit::Tight => {
                eprintln!(
                    "warning: target_chunk_size ({target}) is within {:.0}% of num_ctx ({num_ctx}).\n\
                     \x20        cl100k_base is not this model's tokenizer, so some chunks will\n\
                     \x20        likely exceed the window and hit the on_oversize policy.",
                    vecdb_core::config::TOKENIZER_MARGIN * 100.0
                );
            }
            _ => {}
        }
    }

    // Clamp the chunk byte ceiling to what the model can actually embed.
    //
    // The oversize policy splits large chunks down to exactly `max_chunk_bytes`.
    // If that ceiling is above the model's capacity, the split parts are still
    // too big — the split makes things worse, not better, and the run dies on
    // input it created itself.
    //
    // Clamped rather than rejected: `max_chunk_bytes` is usually derived, and
    // the derivation constant is generous on purpose, so erroring on any
    // derived ceiling above the window would reject the shipped defaults.
    // Capacity is a hard limit; the ceiling is a policy knob; policy yields,
    // and says so.
    //
    // NOT clamped: `target_chunk_size` and `num_ctx`. Those are operator intent,
    // they are checked above, and are used exactly as written.
    let ceiling_cap = context_tokens.map(vecdb_core::config::model_byte_cap);
    let clamp_ceiling = |v: usize| -> usize { ceiling_cap.map_or(v, |cap| v.min(cap)) };

    if let (Some(cap), Some(ctx)) = (ceiling_cap, context_tokens) {
        let configured = resolution.max_chunk_bytes.value;
        if configured > cap {
            // Never gated on `is_interactive`: a caller reading JSON is exactly
            // who needs to know the ceiling it configured is not the one in force.
            eprintln!(
                "note: max_chunk_bytes {configured} exceeds what this model can embed \
                 ({ctx} tokens ~ {cap} bytes); using {cap}.\n\
                 \x20     Oversized chunks split to {cap} instead, which the model accepts.\n\
                 \x20     Set max_chunk_bytes at or below {cap} to silence this."
            );
        }
    }

    let file_detector = Arc::new(HybridDetector::new());
    // Run-level fallback only: the destination's own value is applied per file
    // via `ChunkSpec::pack_target`, because one routed run can span collections
    // configured differently. Baked into the vectors either way — changing it
    // later means a re-ingest.
    let parser_factory = Arc::new(VecqParserFactory::with_pack_target(
        resolution.pack_target_bytes.value,
    ));

    let services = vecdb_core::CoreServices::from_config(
        config,
        file_detector.clone(),
        parser_factory.clone(),
    );
    let core = vecdb_core::Core::new(&resolution, services).await?;

    // Parse metadata
    let mut metadata = std::collections::HashMap::new();
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            metadata.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    // Clamped: see `clamp_ceiling` above. The pipeline must never be handed a
    // ceiling the model cannot embed, because that is what it splits down to.
    let resolved_max_chunk_bytes = Some(clamp_ceiling(resolution.max_chunk_bytes.value));
    let final_chunk_size = args
        .target_chunk_size
        .or(Some(resolution.target_chunk_size.value));
    let final_overlap = args.overlap.or(Some(resolution.chunk_overlap.value));
    let final_respect_gitignore = args.respect_gitignore || config.ingestion.respect_gitignore;

    // ── Multi-path batching ──────────────────────────────────────────
    // When multiple paths are given (glob expansion, CLI listing), we find
    // their common ancestor and pass the specific files as a file_allowlist.
    // This way ONE ingest pipeline discovers + parses + embeds all files,
    // showing proper progress (N/N) and capturing topographic metadata.
    let (ingest_path, file_allowlist, project_root_display) = if args.paths.len() <= 1 {
        // Single path: pass as-is
        let single = args
            .paths
            .first()
            .and_then(|p| p.to_str())
            .unwrap_or(".")
            .to_string();
        let single_display = single.clone();
        (single, None, single_display)
    } else {
        // Multiple paths: compute common ancestor
        let resolved: Vec<PathBuf> = args
            .paths
            .iter()
            .filter_map(|p| std::fs::canonicalize(p).ok())
            .collect();

        let common =
            common_ancestor(&resolved).unwrap_or_else(|| std::path::Path::new(".").to_path_buf());

        let allowlist: Vec<String> = resolved
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        let display = common.to_string_lossy().to_string();
        if OUTPUT.is_interactive {
            let n = allowlist.len();
            println!("Batch ingest: {} files under {}", n, display);
        }
        let display_for_meta = display.clone();
        (display, Some(allowlist), display_for_meta)
    };

    // Discovered above, before resolution — reused here rather than re-read, so
    // the file that supplied the collection is the same one supplying routes.
    let (rc_path, rc, extra_passes) = match rc_result {
        Some((rc_path, rc)) => {
            if OUTPUT.is_interactive {
                println!(
                    "Found .vecdbrc at {} with {} route(s). Enabling per-file routing.",
                    rc_path.display(),
                    rc.routes.len()
                );
            }

            // ONCE if any route disagrees with `-c`, never once per route.
            //
            // Distinct from the cross-space split below: this is about intent
            // (`-c` says one thing, the file says another), not about
            // embedding-space correctness.
            let has_mismatch = rc.routes.iter().any(|r| r.collection != collection);
            if has_mismatch && OUTPUT.is_interactive {
                eprintln!(
                    "Warning: .vecdbrc routes to different collection. Verify profile matches '-c {collection}'"
                );
            }
            // A route may send files to another COLLECTION, but this run has a
            // single embedder and a single store — resolved once, before any
            // file is read. `route_chunking` below resolves chunk size per
            // routed collection; the embedder and the store are NOT resolved
            // that way and cannot be, because the pipeline holds one `Core`.
            //
            // So a route naming a collection whose profile resolves to a
            // different embedder or store writes THIS run's vectors under that
            // name: wrong model, wrong width, wrong endpoint. When the target
            // does not exist yet it is CREATED at the wrong dimension, which
            // the embedding-space guard cannot catch because there is no
            // genesis to compare against. That is how a 768-dim `notes-archive`
            // appeared on the local store while the real 2560-dim one lived
            // elsewhere (2026-256).
            //
            // Refuse instead of corrupting. The operator's `.vecdbrc` is not
            // wrong — the feature cannot honour it yet. Until routing resolves
            // per-collection embedders, split the run: one invocation per
            // target collection, each with `-c` and its own profile.
            // A route may send files to another COLLECTION whose profile
            // resolves to a different model or a different store. One pass
            // cannot serve those — it holds a single embedder and a single
            // backend, resolved before any file is read — so routing a file
            // elsewhere would embed it with the wrong model and write it to the
            // wrong endpoint, CREATING the target at the wrong dimension when it
            // does not yet exist. The embedding-space guard cannot catch that:
            // there is no genesis to compare against. It is how a 768-dim
            // `notes-archive` came to sit on the local store while the real 2560-dim
            // one lived elsewhere (2026-256).
            //
            // So the run is split: one pass per destination, each with that
            // collection's own resolution, each ignoring the others' files.
            // `route_chunking` already resolves chunk size this way; this
            // extends the same idea to the model and the store.
            let mut extra_passes: Vec<String> = Vec::new();
            for r in &rc.routes {
                if r.collection == collection || r.collection.is_empty() {
                    continue;
                }
                if let Ok(other) = config.resolve(profile_name, Some(&r.collection)) {
                    // Compare the SPACE, not the embedder name: `nomic_gpu` and
                    // `nomic_cpu` are two names for one model and produce
                    // interchangeable vectors.
                    let same_space = other.embedder.model == resolution.embedder.model
                        && other.qdrant_url == resolution.qdrant_url;
                    if !same_space {
                        extra_passes.push(r.collection.clone());
                    }
                }
            }
            extra_passes.sort();
            extra_passes.dedup();

            if OUTPUT.is_interactive && !extra_passes.is_empty() {
                println!(
                    "Routing spans {} embedding spaces; running one pass per destination: {}, {}",
                    extra_passes.len() + 1,
                    collection,
                    extra_passes.join(", ")
                );
            }
            (Some(rc_path), Some(rc), extra_passes)
        }
        None => (None, None, Vec::new()),
    };

    let vecdbrc_root = rc_path
        .as_ref()
        .and_then(|p| VecdbRc::project_root(p))
        .unwrap_or_else(|| std::path::Path::new(&ingest_path))
        .to_path_buf();

    // Build IngestionOptions with optional file_allowlist
    let opts = vecdb_core::ingestion::IngestionOptions {
        path: ingest_path.clone(),
        collection: collection.to_string(),
        file_allowlist,
        project_root: Some(project_root_display),
        vecdbrc_routes: rc.as_ref().map(|r| r.routes.clone()),
        vecdbrc_root: Some(vecdbrc_root),
        target_chunk_size: final_chunk_size
            .unwrap_or(vecdb_core::config::DEFAULT_TARGET_CHUNK_SIZE),
        max_chunk_bytes: resolved_max_chunk_bytes,
        pack_target_bytes: Some(resolution.pack_target_bytes.value),
        on_oversize: config.resolve_oversize_policy().value,
        // Chunk parameters per routed destination.
        //
        // Resolved here rather than in the ingestion layer because working out a
        // collection's chunk size needs `Config`, and `vecdb-core::ingestion`
        // deliberately does not depend on it. Without this, a `.vecdbrc` fanning
        // across collections chunks them all identically — and chunk size is
        // baked into the vectors at ingest, so the only repair is a re-ingest.
        route_chunking: rc
            .as_ref()
            .map(|r| {
                r.routes
                    .iter()
                    .map(|route| route.collection.clone())
                    .chain(std::iter::once(collection.to_string()))
                    .filter(|c| !c.is_empty())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .filter_map(|coll| {
                        // A route naming a collection with no config entry falls
                        // through to the run's own parameters, which is the same
                        // answer as before and needs no map entry.
                        let r = config.resolve(profile_name, Some(&coll)).ok()?;
                        Some((
                            coll,
                            vecdb_core::ingestion::options::ChunkSpec {
                                target_chunk_size: r.target_chunk_size.value,
                                chunk_overlap: r.chunk_overlap.value,
                                // Per-route ceiling, clamped the same way: a
                                // route naming a collection with a looser
                                // ceiling must not smuggle one past the model.
                                max_chunk_bytes: Some(clamp_ceiling(r.max_chunk_bytes.value)),
                                // The knob that actually governs AST content.
                                // Resolved per destination for the same reason
                                // as the rest: chunk size belongs to the
                                // collection, not to the run.
                                pack_target_bytes: Some(r.pack_target_bytes.value),
                            },
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        chunk_overlap: final_overlap.unwrap_or(50),
        respect_gitignore: final_respect_gitignore,
        ignore_vectorignore: args.ignore_vectorignore,
        strategy: "recursive".to_string(),
        tokenizer: "cl100k_base".to_string(),
        git_ref: None,
        extensions: args.extensions.clone(),
        excludes: args.excludes.clone(),
        dry_run: args.dry_run,
        metadata: if metadata.is_empty() {
            None
        } else {
            Some(metadata.clone())
        },
        // ingest_with_options does NOT pull from Core; caller must supply rules explicitly.
        path_rules: config.ingestion.path_rules.clone(),
        max_concurrent_requests: args.concurrency.unwrap_or(4),
        gpu_batch_size: args.gpu_concurrency.unwrap_or(2),
        quantization: resolution.quantization.clone(),
        allow_quantization_delta: args.allow_quantization_delta,
        // Only restrict when the run is genuinely split. With one destination
        // this stays `None`, so the common path is byte-for-byte unchanged.
        only_collection: if extra_passes.is_empty() {
            None
        } else {
            Some(collection.to_string())
        },
        route_default_collection: Some(collection.to_string()),
    };

    let opts_template = opts.clone();

    tokio::select! {
        res = core.ingest_with_options(opts, None) => {
            res?;
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nCancelled by user.");
            // 130 = 128 + SIGINT, the shell convention. Exiting 0 told every
            // caller the work had SUCCEEDED, so a `for` loop over directories
            // saw a clean result and advanced to the next one — which is why
            // Ctrl-C appeared not to stop `vecdb.sh` and it had to be killed by
            // PID. Cancelled is not done.
            std::process::exit(130);
        }
    }

    // One further pass per destination that needs a different model or store.
    //
    // Each builds its own `Core` from that collection's resolution, so the
    // embedder and endpoint match the collection it writes. Files belonging to
    // the other destinations are ignored by `only_collection`, so every file is
    // handled exactly once across the passes.
    for target in &extra_passes {
        let target_res = config.resolve(profile_name, Some(target))?;
        if OUTPUT.is_interactive {
            println!(
                "\n── pass: '{}' via {} at {}",
                target, target_res.embedder.model, target_res.qdrant_url
            );
        }

        let services = vecdb_core::CoreServices::from_config(
            config,
            std::sync::Arc::new(HybridDetector::new()),
            std::sync::Arc::new(VecqParserFactory::with_pack_target(
                config.ingestion.pack_target_bytes,
            )),
        );
        let target_core = vecdb_core::Core::new(&target_res, services).await?;

        let mut target_opts = opts_template.clone();
        // `collection` becomes this destination: it is what the embedding-space
        // guard validates against this pass's embedder. `route_default_collection`
        // stays the original, so unrouted files still belong to the first pass
        // and are not re-ingested here.
        target_opts.collection = target.clone();
        target_opts.only_collection = Some(target.clone());
        target_opts.target_chunk_size = target_res.target_chunk_size.value;
        target_opts.chunk_overlap = target_res.chunk_overlap.value;
        target_opts.quantization = target_res.quantization.clone();

        tokio::select! {
            res = target_core.ingest_with_options(target_opts, None) => {
                res?;
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nCancelled by user.");
                // See above: cancelled is not done. 130 = 128 + SIGINT.
                std::process::exit(130);
            }
        }
    }

    if OUTPUT.is_interactive && !args.dry_run {
        println!("Ingestion complete.");
    }

    Ok(())
}

async fn run_stdin(
    args: IngestArgs,
    config: &Config,
    _profile_name: &str,
    resolution: &vecdb_core::config::Resolution,
    collection: &str,
) -> anyhow::Result<()> {
    if OUTPUT.is_interactive {
        println!("Ingesting from stdin into collection: {}...", collection);
    }

    let mut buffer = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)?;

    if buffer.trim().is_empty() {
        eprintln!("Warning: Empty input from stdin.");
        return Ok(());
    }

    let file_detector = Arc::new(HybridDetector::new());
    // Run-level fallback only: the destination's own value is applied per file
    // via `ChunkSpec::pack_target`, because one routed run can span collections
    // configured differently. Baked into the vectors either way — changing it
    // later means a re-ingest.
    let parser_factory = Arc::new(VecqParserFactory::with_pack_target(
        resolution.pack_target_bytes.value,
    ));

    let services = vecdb_core::CoreServices::from_config(
        config,
        file_detector.clone(),
        parser_factory.clone(),
    );
    let core = vecdb_core::Core::new(resolution, services).await?;

    let mut metadata = std::collections::HashMap::new();
    for item in &args.metadata {
        if let Some((key, value)) = item.split_once('=') {
            metadata.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
    metadata
        .entry("source".to_string())
        .or_insert(serde_json::Value::String("stdin".to_string()));

    // Same clamp as the directory path. stdin is not a special case: the model
    // limit applies to whatever produced the bytes.
    let stdin_cap = if resolution.is_ollama() {
        Some(resolution.num_ctx.value)
    } else {
        vecdb_core::embedders::local::fastembed_context_length(&resolution.embedder.model)
            .map(|n| n as usize)
    }
    .map(vecdb_core::config::model_byte_cap);
    let resolved_max_chunk_bytes =
        Some(stdin_cap.map_or(resolution.max_chunk_bytes.value, |cap| {
            resolution.max_chunk_bytes.value.min(cap)
        }));
    let final_chunk_size = args
        .target_chunk_size
        .or(Some(resolution.target_chunk_size.value));
    let final_overlap = args.overlap.or(Some(resolution.chunk_overlap.value));

    tokio::select! {
        res = core.ingest_content(&buffer, metadata, collection, final_chunk_size, resolved_max_chunk_bytes, final_overlap, resolution.quantization.clone(), None) => {
            res?;
            println!("Ingestion complete.");
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nCancelled by user.");
            // Same contract as the other cancellation sites: cancelled is not
            // done, and `Ok(())` here would exit 0 and tell the caller it was.
            std::process::exit(130);
        }
    }
    Ok(())
}

/// Find the common ancestor directory of a set of paths.
/// Returns the longest shared prefix path component.
fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        return paths[0].parent().map(|p| p.to_path_buf());
    }

    // Collect all ancestors of the first path
    let first = &paths[0];
    let ancestors: Vec<&std::path::Path> = first.ancestors().collect();

    // Find the longest ancestor shared by all paths
    for ancestor in ancestors {
        if paths.iter().all(|p| p.starts_with(ancestor)) {
            return Some(ancestor.to_path_buf());
        }
    }

    // Fall back to current directory
    Some(std::path::Path::new(".").to_path_buf())
}
