use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "docsize")]
#[command(version = "1.0.0")]
#[command(about = "Contextualized prompt generator for vecdb/vecq")]
pub struct Args {
    /// The query or prompt to send
    #[arg(index = 1)]
    pub query: Option<String>,

    /// Target directory (defaults to current directory)
    #[arg(short, long, default_value = ".")]
    pub dir: PathBuf,

    /// Omit providing directory information
    #[arg(short, long)]
    pub no_context: bool,

    /// Append to the current conversation session
    #[arg(short, long)]
    pub append: bool,

    /// Specify the LLM model to use
    /// Specify the LLM model to use (pass no value to select interactively)
    #[arg(short, long, num_args = 0..=1, default_missing_value = "_INTERACTIVE_", require_equals = true)]
    pub model: Option<String>,

    /// Show the final prompt being sent to the LLM (for debugging)
    #[arg(long, short = 'v', alias = "verbose")]
    pub debug: bool,

    /// Enable Smart Routing (detect facets from query)
    #[arg(short, long)]
    pub smart: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Show manual page
    Man {
        /// Show agent-optimized documentation
        #[arg(long)]
        agent: bool,
    },
}
