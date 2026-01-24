// CLI module for vecq
// Provides the command-line interface functionality

pub mod args;
pub mod elements;
pub mod output;
pub mod run;

// Re-export main functions for use by main.rs
pub use args::{get_informed_command, handle_completions, Args, Commands};
pub use elements::handle_elements_command;
pub use run::{format_user_error, handle_subcommand, print_available_formats, print_query_explanation, print_supported_types, run_main_command};