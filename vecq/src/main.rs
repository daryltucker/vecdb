// PURPOSE:
//   Command-line interface for vecq - the "jq for source code" tool.
//   Provides user-friendly CLI that makes document parsing and querying accessible
//   to developers and integrates seamlessly with Unix pipelines. Essential for
//   vecq's adoption as it's the primary interface most users will interact with.
//
// REQUIREMENTS:
//   User-specified:
//   - Must support `vecq <file> [query]` syntax for direct file querying
//   - Must support `vecq <directory> [query]` for batch processing multiple files
//   - Must provide `vecq --convert <file>` to output raw JSON without querying
//   - Must include comprehensive help via `vecq --help` and `vecq man`
//   - Must support various output formats (JSON, grep-compatible, human-readable)
//   - Must integrate with Unix pipelines and standard tools
//
//   Implementation-discovered:
//   - Requires clap for robust argument parsing and help generation
//   - Must handle async operations for file I/O and parsing
//   - Needs proper error handling and user-friendly error messages
//   - Must support configuration files for user preferences
//   - Requires progress indicators for long-running batch operations
//
// IMPLEMENTATION RULES:
//   1. Use clap derive API for clean argument parsing and help generation
//      Rationale: Provides consistent CLI experience and automatic help text
//
//   2. Handle all errors gracefully with user-friendly messages
//      Rationale: Technical error messages confuse users and hurt adoption
//
//   3. Support both single file and batch processing modes
//      Rationale: Users need both quick single-file queries and bulk analysis
//
//   4. Integrate seamlessly with Unix pipelines and standard tools
//      Rationale: Essential for vecq's value proposition as Unix-friendly tool
//
//   5. Provide progress feedback for long-running operations
//      Rationale: Users need to know the tool is working on large datasets
//
//   Critical:
//   - DO NOT break Unix pipeline compatibility with output format changes
//   - DO NOT expose internal error details to end users
//   - ALWAYS provide helpful suggestions for common user mistakes
//
// USAGE:
//   # Basic file querying
//   vecq src/main.rs '.functions[] | select(.visibility == "pub")'
//
//   # Convert to JSON without querying
//   vecq --convert README.md
//
//   # Batch processing with grep-compatible output
//   vecq src/ '.functions[]' --grep-format | grep "TODO"
//
//   # Human-readable output
//   vecq src/lib.rs '.structs[]' --format human
//
//   # Pipeline integration
//   find . -name "*.rs" | xargs vecq --grep-format | grep "pub fn" | cut -d: -f1 | sort -u
//
// SELF-HEALING INSTRUCTIONS:
//   When adding new CLI options:
//   1. Add new field to Args struct with appropriate clap attributes
//   2. Update help text and examples in clap attributes
//   3. Add handling logic in main() function
//   4. Update error handling for new option validation
//   5. Add integration tests for new CLI functionality
//   6. Update user documentation and examples
//
//   When modifying output formats:
//   1. Ensure backward compatibility with existing pipelines
//   2. Test with common Unix tools (grep, awk, sed, cut, sort)
//   3. Update format documentation and examples
//   4. Add regression tests for format changes
//   5. Consider deprecation warnings for breaking changes
//
// RELATED FILES:
//   - src/lib.rs - Library functions used by CLI
//   - Cargo.toml - CLI dependencies and binary configuration
//   - README.md - User documentation and examples
//   - tests/integration/ - CLI integration tests
//   - examples/ - Usage examples for different scenarios
//
// MAINTENANCE:
//   Update when:
//   - New library functionality needs CLI exposure
//   - User feedback indicates CLI usability issues
//   - New output formats or options are requested
//   - Integration with Unix tools needs improvement
//   - Performance optimization affects user experience
//
// Last Verified: 2025-12-31


use clap::FromArgMatches;
use std::process;

mod cli;
use cli::*;
mod man_cmd;

/// vecq - jq for source code
///
/// Convert any structured document to queryable JSON and query with jq syntax.
/// Supports Rust, Python, Markdown, C/C++, CUDA, Go, and Bash files.


#[tokio::main]
async fn main() {
    let cmd = cli::get_informed_command();
    let matches = cmd.get_matches();
    let args = cli::Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    // Handle informational commands first
    if args.list_types {
        cli::print_supported_types();
        return;
    }

    if args.list_formats {
        cli::print_available_formats();
        return;
    }

    // Handle subcommands
    if let Some(command) = args.command {
        match command {
            cli::Commands::Completions { shell } => {
                cli::handle_completions(shell);
                return;
            }
            cli::Commands::Elements => {
                if let Err(e) = cli::handle_elements_command(&matches).await {
                    eprintln!("Error: {}", cli::format_user_error(&e));
                    process::exit(1);
                }
            }
            _ => match cli::handle_subcommand(command).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error: {}", cli::format_user_error(&e));
                    process::exit(1);
                }
            },
        }
        return;
    }

    // Resolve query from flag or positional argument
    let query = args.query_flag.clone().or(args.query_positional.clone());

    // Handle query validation/explanation
    if let Some(ref query) = query {
        if args.validate {
            match vecq::validate_query(query) {
                Ok(()) => {
                    println!("Query syntax is valid");
                    return;
                }
                Err(e) => {
                    eprintln!("Query validation failed: {}", cli::format_user_error(&e));
                    process::exit(1);
                }
            }
        }

        if args.explain {
            match vecq::explain_query(query) {
                Ok(explanation) => {
                    print_query_explanation(&explanation);
                    return;
                }
                Err(e) => {
                    eprintln!("Query explanation failed: {}", cli::format_user_error(&e));
                    process::exit(1);
                }
            }
        }
    }

    // Main processing
    match cli::run_main_command(args).await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", cli::format_user_error(&e));
            process::exit(1);
        }
    }
}
