//! DOCS: docs/CLI.md
//! COMPLIANCE: tests/tier2_cli_compliance.py
/*
 * PURPOSE:
 *   Main entry point for vecdb-cli.
 *   Parses arguments and dispatches to subcommands.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - CLI structure (init, ingest, search, man)
 *   - Config file with profiles (User Prompt)
 *   - Default profile capability
 *
 * IMPLEMENTATION RULES:
 *   1. Use `clap` derive pattern
 *      Rationale: Type-safe argument parsing.
 *   2. Load Config early
 *      Rationale: Fail fast if config is corrupt (unless init).
 */

mod cli;
mod commands;
mod vecq_adapter;

// SAFETY: Jemalloc is configured as the global allocator for Linux targets to reduce fragmentation
// in long-running async server workloads (ingestion/search).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum QuantizationArg {
    Scalar,
    Binary,
    None,
}

impl From<QuantizationArg> for vecdb_core::config::QuantizationType {
    fn from(val: QuantizationArg) -> Self {
        match val {
            QuantizationArg::Scalar => vecdb_core::config::QuantizationType::Scalar,
            QuantizationArg::Binary => vecdb_core::config::QuantizationType::Binary,
            QuantizationArg::None => vecdb_core::config::QuantizationType::None,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging (clean production default)
    vecdb_common::logging::init_logging();

    let result = cli::run().await;

    // Handle SIGPIPE (Broken Pipe) gracefully
    if let Err(err) = result {
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
        }
        return Err(err);
    }
    Ok(())
}
