use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::Path;
use walkdir::WalkDir;
use xshell::{cmd, Shell};

#[derive(Parser)]
#[command(name = "xtask")]
#[command(about = "Automation tasks for vecdb-mcp", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run all tests with strict isolation (Nextest)
    Test {
        /// Run with coverage (LSan)
        #[arg(long)]
        coverage: bool,
    },
    /// Run CI pipeline locally
    Ci,
    /// Enforce architecture policies (Mirror Policy)
    Lint,
    /// Regenerate the configuration reference in docs/CONFIG.md from the
    /// config structs' own doc comments.
    GenConfigDocs {
        /// Report whether the file is stale instead of rewriting it.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let sh = Shell::new()?;

    match cli.command {
        Commands::Test { coverage } => run_test(&sh, coverage)?,
        Commands::Ci => run_ci(&sh)?,
        Commands::Lint => run_lint(&sh)?,
        Commands::GenConfigDocs { check } => gen_config_docs(check)?,
    }

    Ok(())
}

fn run_test(sh: &Shell, coverage: bool) -> Result<()> {
    println!("Running tests with cargo-nextest...");

    // Ensure nextest is installed
    if cmd!(sh, "cargo nextest --version").read().is_err() {
        eprintln!("cargo-nextest not found. Installing...");
        cmd!(sh, "cargo install cargo-nextest").run()?;
    }

    let _coverage_flag = if coverage { "true" } else { "false" }; // Placeholder logic
    if coverage {
        println!("Note: LSan coverage requires nightly and RUSTFLAGS configuration which will be added in Phase 3.");
    }

    cmd!(sh, "cargo nextest run --profile default").run()?;
    Ok(())
}

fn run_lint(_sh: &Shell) -> Result<()> {
    println!("Running Architecture Lints...");

    // 1. The Mirror Policy: src/parsers/X.rs -> tests/fixture_X.rs
    let parsers_dir = Path::new("vecdb-core/src/parsers");
    let tests_dir = Path::new("vecdb-core/tests");

    if !parsers_dir.exists() {
        println!("Skipping Mirror Policy Check: vecdb-core/src/parsers not found");
        return Ok(());
    }

    let mut violations = Vec::new();

    for entry in WalkDir::new(parsers_dir).max_depth(1) {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            let stem = path.file_stem().unwrap().to_string_lossy();
            if stem == "mod" {
                continue;
            }

            let expected_test = tests_dir.join(format!("fixture_{}.rs", stem));
            if !expected_test.exists() {
                violations.push(format!(
                    "Missing integration test for parser '{}'. Expected: {}",
                    stem,
                    expected_test.display()
                ));
            }
        }
    }

    if !violations.is_empty() {
        eprintln!("\n❌ Mirror Policy Violations:");
        for v in violations {
            eprintln!("   - {}", v);
        }
        anyhow::bail!("Architecture Lint Failed: Parsers must have corresponding 'fixture_<name>.rs' integration tests.");
    }

    println!("✅ Mirror Policy Passed");
    Ok(())
}

fn run_ci(sh: &Shell) -> Result<()> {
    println!("Running CI pipeline...");

    // 0. Architecture Lint
    run_lint(sh)?;

    // 1. Check formatting
    cmd!(sh, "cargo fmt --all -- --check").run()?;
    // 2. Clippy
    cmd!(sh, "cargo clippy --all-targets -- -D warnings").run()?;
    // 3. Tests (Unit + Integration)
    // Use CI profile for stricter timeouts
    cmd!(sh, "cargo nextest run --profile ci").run()?;

    println!("CI Pipeline Passed! 🚀");
    Ok(())
}

/// Regenerate `docs/CONFIG.md`'s reference tables from `vecdb-core`'s config
/// structs.
///
/// The reference is derived rather than written because the two used to drift
/// silently: the compliance check only verified that a field name appeared in
/// the file, so a wrong type, a wrong default, or a description contradicting
/// the code all passed. `--check` is what the test suite runs.
fn gen_config_docs(check: bool) -> Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("docs/CONFIG.md");

    // A blank description is an undocumented field wearing a table row.
    let missing = vecdb_core::config_docs::undocumented();
    if !missing.is_empty() {
        anyhow::bail!(
            "config fields have no doc comment, so their reference rows would be blank:\n  {}\n\n  \
             Add a `///` comment in vecdb-core/src/config.rs — that comment IS the docs.",
            missing.join("\n  ")
        );
    }

    let existing = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let updated = vecdb_core::config_docs::splice(&existing)?;

    if check {
        if existing != updated {
            anyhow::bail!(
                "docs/CONFIG.md is stale.\n\n  \
                 The reference tables no longer match the config structs.\n  \
                 Regenerate: cargo run -p xtask -- gen-config-docs"
            );
        }
        println!("docs/CONFIG.md is up to date.");
        return Ok(());
    }

    if existing == updated {
        println!("docs/CONFIG.md already up to date.");
    } else {
        std::fs::write(&path, updated)?;
        println!("Regenerated {}", path.display());
    }
    Ok(())
}
