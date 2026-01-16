mod types;
mod strategy;

use clap::Parser;
use std::path::PathBuf;
use serde_json::Value;
use vecdb_common::{InputContext, OutputContext};
use anyhow::Result;
use std::io::{Read, Write};

// Import strategy handlers
use strategy::stream::process_stream;
use strategy::state::{process_state, FileSystemSnapshotLoader};

#[derive(Parser, Debug)]
#[command(name = "vecdb-asm")]
#[command(about = "Stateful Assembler for vecdb - merges and deduplicates temporal knowledge streams")]
struct Args {
    /// Strategy to use: 'stream' (consolidate logs) or 'state' (reduce snapshots)
    #[arg(short, long, default_value = "stream")]
    strategy: String,

    /// Disable deduplication (Stream strategy only)
    #[arg(long)]
    no_dedupe: bool,

    /// Stitch overlapping fragments (Stream strategy only)
    #[arg(long)]
    stitch: bool,

    /// Detect divergent timelines (State strategy only)
    #[arg(long)]
    detect_timelines: bool,

    /// Input file (optional, defaults to stdin)
    #[arg(value_name = "INPUT")]
    input: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Parser, Debug)]
enum Commands {
    /// Display manual entry
    Man {
        /// Display agent-optimized manual
        #[arg(long)]
        agent: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Check for subcommand first (e.g. `man`)
    if let Some(Commands::Man { agent }) = args.command {
        if agent {
            print!("{}", include_str!("docs/man_agent.md"));
        } else {
            print!("{}", include_str!("docs/man_human.md"));
        }
        return Ok(());
    }

    let input_ctx = InputContext::detect();
    let _output_ctx = OutputContext::detect();

    if args.verbose {
        eprintln!("Strategy: {}", args.strategy);
    }

    // 1. Read Input
    let mut buffer = String::new();
    if let Some(path) = &args.input {
        let mut file = std::fs::File::open(path)?;
        file.read_to_string(&mut buffer)?;
    } else if input_ctx.has_piped_data {
        std::io::stdin().read_to_string(&mut buffer)?;
    } else {
        // No input file, no piped data -> Show Help or Error
        // No input file, no piped data -> Show Help or Error
        eprintln!("Error: No input provided. Pipe data via stdin or provide a file path.");
        return Ok(());
    }

    if buffer.trim().is_empty() {
        println!("[]");
        return Ok(());
    }

    let input_val: Value = if let Ok(val) = serde_json::from_str(&buffer) {
        val
    } else {
        // Try parsing as NDJSON/JSONL
        let records: Vec<Value> = buffer
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        
        if records.is_empty() {
            eprintln!("Error: Could not parse input as JSON or JSONL.");
            return Ok(());
        }
        Value::Array(records)
    };

    // 2. Select and Execute Strategy
    let result = match args.strategy.as_str() {
        "stream" => {
            if args.verbose { eprintln!("Executing Stream Strategy..."); }
            process_stream(input_val, args.no_dedupe, args.stitch)?
        },
        "state" => {
            if args.verbose { eprintln!("Executing State Strategy..."); }
            let loader = FileSystemSnapshotLoader;
            process_state(input_val, &loader, args.detect_timelines)?
        },
        _ => anyhow::bail!("Unknown strategy: {}", args.strategy),
    };

    // 3. Write Output
    let output_str = serde_json::to_string_pretty(&result)?;
    std::io::stdout().write_all(output_str.as_bytes())?;
    println!(); // Trailing newline

    Ok(())
}

