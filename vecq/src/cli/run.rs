// Main command execution logic for vecq CLI
// Handles the core query processing and input/output flow

use std::path::PathBuf;
use tokio::fs;
use std::io::BufRead;
use vecq::{available_output_formats, supported_file_types, FormatOptions, JqQueryEngine, QueryEngine, VecqError, VecqResult};
use vecdb_common::output::{OutputContext, OutputFormat};

use super::args::{Args, Commands};
use super::output::{extract_json_from_input, process_json_value};

/// Execute the main vecq command with the given arguments
pub async fn run_main_command(mut args: Args) -> VecqResult<()> {
    // Resolve query from file, flag, or positional
    let query_string = if let Some(path) = &args.from_file {
        fs::read_to_string(path).await.map_err(|e| {
            VecqError::IoError(std::io::Error::new(
                e.kind(),
                format!("Failed to read query file: {}", e),
            ))
        })?
    } else {
        match args.query_flag.clone().or(args.query_positional.take()) {
            Some(q) => q,
            None => "".to_string(), // Will be handled as empty/identity if needed, or ignored if not querying
        }
    };

    // Configure query engine
    let mut engine = JqQueryEngine::new();
    for path in &args.library_path {
        engine.add_library_path(path.clone());
    }

    // Determine output format
    let output_format = if args.grep_format {
        "grep".to_string()
    } else if args.json {
        "json".to_string()
    } else if args.human {
        "human".to_string()
    } else if let Some(ref fmt) = args.format {
        fmt.clone()
    } else {
        // Smart Default
        // If we are writing to TTY, we prefer human-readable output (Markdown/Text)
        // If we are piping, we default to JSON for machine consumption
        let ctx = OutputContext::detect();

        match ctx.resolve_format() {
            OutputFormat::Markdown | OutputFormat::Text => "human".to_string(),
            OutputFormat::Grep => "grep".to_string(),
            OutputFormat::Json => "json".to_string(),
        }
    };

    // Auto-enable raw output if "human" format is selected, to avoid quoting strings in tables
    let raw_output = if output_format == "human" {
        true
    } else {
        args.raw_output
    };

    // Create format options
    let format_options = FormatOptions {
        pretty_print: args.pretty && !args.compact,
        compact: args.compact,
        grep_compatible: args.grep_format,
        color_output: args
            .color
            .unwrap_or_else(|| vecdb_common::OUTPUT.use_color()),
        include_line_numbers: true,
        include_file_paths: true,
        max_width: if output_format == "human" {
            Some(120)
        } else {
            None
        },
        custom_format: None,

        raw_output,
    };

    // Resolve inputs: if empty and stdin has data, use stdin
    let mut resolved_inputs = args.inputs.clone();
    if resolved_inputs.is_empty() && vecdb_common::INPUT.has_piped_data {
        resolved_inputs.push(PathBuf::from("-"));
    }

    if resolved_inputs.is_empty() {
        return Err(VecqError::ConfigError {
            message: "No input provided. Usage: vecq <file> [query] or pipe data: cat file | vecq"
                .to_string(),
        });
    }

    // Setup Buffered Output
    let stdout = std::io::stdout();
    let mut handle = std::io::BufWriter::new(stdout.lock());

    if args.slurp {
        let mut all_values = Vec::new();
        for input in resolved_inputs {
            all_values.extend(extract_json_from_input(&input, &(&args).into()).await?);
        }
        let slurp_value = serde_json::Value::Array(all_values);
        // Synchronous call (no await)
        process_json_value(
            slurp_value,
            &query_string,
            &engine,
            &output_format,
            &format_options,
            &mut handle,
        )?;
    } else {
        for input in resolved_inputs {
            let values = extract_json_from_input(&input, &(&args).into()).await?;
            for val in values {
                // Synchronous call (no await)
                process_json_value(
                    val,
                    &query_string,
                    &engine,
                    &output_format,
                    &format_options,
                    &mut handle,
                )?;
            }
        }
    }

    // Ensure we flush at the end
    use std::io::Write;
    handle.flush().map_err(VecqError::IoError)?;

    Ok(())
}

/// Handle subcommands (non-main query commands)
pub async fn handle_subcommand(command: Commands) -> VecqResult<()> {
    match command {
        Commands::Doc { input } => {
            let options = super::args::ParseOptions {
                file_type: None,
                context_lines: 0,
                verbose: false,
                recursive: false,
                max_depth: None,
                enable_usages: false,
            };
            let values = extract_json_from_input(&input, &options).await?;
            let engine = JqQueryEngine::new_hermetic();

            // For now, simpler than full iterator: just process each file
            for val in values {
                let results = engine.execute_query(&val, "markdown")?;
                // If the result is a string, print it raw. JSON otherwise.
                for result in results {
                    match result {
                        serde_json::Value::String(s) => println!("{}", s),
                        _ => println!("{}", result),
                    }
                }
            }
            Ok(())
        }
        Commands::Man { agent, command } => {
            use crate::man_cmd;
            if let Err(e) = man_cmd::run(agent, command) {
                eprintln!("Error displaying manual: {}", e);
            }
            Ok(())
        }
        Commands::Suggest { description } => {
            print_query_suggestions(&description);
            Ok(())
        }
        Commands::Elements => Ok(()), // Handled in main directly to support dynamic sub-subcommands
        Commands::Completions { .. } => unreachable!("Handled in main"),
        Commands::Syntax { input, language } => {
            use std::io::Read;
            use syntect::easy::HighlightFile;
            use syntect::highlighting::{Style, ThemeSet};
            use syntect::parsing::SyntaxSet;
            use syntect::util::as_24_bit_terminal_escaped;

            let ss = SyntaxSet::load_defaults_newlines();
            let ts = ThemeSet::load_defaults();

            // Use a dark theme by default, or fallback
            // let theme = &ts.themes["base16-ocean.dark"];
            let theme = ts
                .themes
                .get("base16-ocean.dark")
                .unwrap_or_else(|| ts.themes.values().next().unwrap());

            let manual_syntax = if let Some(lang) = language {
                ss.find_syntax_by_token(&lang)
                    .or_else(|| ss.find_syntax_by_extension(&lang))
            } else {
                None
            };

            if let Some(path) = input {
                // If manual language specified, use HighlightLines instead of HighlightFile to force it
                if let Some(syntax) = manual_syntax {
                    use std::fs::File;
                    use std::io::BufReader;

                    let mut h = syntect::easy::HighlightLines::new(syntax, theme);
                    let file = File::open(path)?;
                    let mut reader = BufReader::new(file);
                    let mut line = String::new();
                    while reader.read_line(&mut line)? > 0 {
                        let regions: Vec<(Style, &str)> =
                            h.highlight_line(&line, &ss)
                                .map_err(|e| VecqError::ConfigError {
                                    message: e.to_string(),
                                })?;
                        print!("{}", as_24_bit_terminal_escaped(&regions[..], true));
                        line.clear();
                    }
                } else {
                    let mut highlighter = HighlightFile::new(path, &ss, theme)
                        .map_err(|e| VecqError::IoError(std::io::Error::other(e)))?;
                    let mut line = String::new();
                    while highlighter.reader.read_line(&mut line)? > 0 {
                        let regions: Vec<(Style, &str)> = highlighter
                            .highlight_lines
                            .highlight_line(&line, &ss)
                            .map_err(|e| VecqError::ConfigError {
                                message: e.to_string(),
                            })?;
                        print!("{}", as_24_bit_terminal_escaped(&regions[..], true));

                        line.clear();
                    }
                }
            } else {
                // Handle stdin
                let mut content = String::new();
                std::io::stdin().read_to_string(&mut content)?;

                // Try to detect syntax from content or default to plain text (or guess)
                let syntax = manual_syntax
                    .or_else(|| ss.find_syntax_by_first_line(&content))
                    .unwrap_or_else(|| ss.find_syntax_plain_text());
                let mut h = syntect::easy::HighlightLines::new(syntax, theme);

                for line in content.lines() {
                    let regions: Vec<(Style, &str)> =
                        h.highlight_line(line, &ss)
                            .map_err(|e| VecqError::ConfigError {
                                message: e.to_string(),
                            })?;
                    println!("{}", as_24_bit_terminal_escaped(&regions[..], true));
                }
            }
            // Reset colors
            println!("\x1b[0m");
            Ok(())
        }
        Commands::ListFilters => {
            print_available_filters();
            Ok(())
        }
    }
}

/// Print supported file types
pub fn print_supported_types() {
    println!("Supported file types:");
    for file_type in supported_file_types() {
        let extensions = file_type.file_extensions().join(", ");
        println!("  {} ({})", file_type, extensions);
    }
}

/// Print available output formats
pub fn print_available_formats() {
    println!("Available output formats:");
    for format in available_output_formats() {
        let description = match format.as_str() {
            "json" => "JSON output (default)",
            "grep" => "Grep-compatible format (filename:line:content)",
            "human" => "Human-readable table format",
            _ => "Custom format",
        };
        println!("  {} - {}", format, description);
    }
}

/// Print query suggestions for natural language input
fn print_query_suggestions(description: &str) {
    println!("Query suggestions for: \"{}\"", description);

    let registry = vecq::SchemaRegistry::new();
    let mut suggestions = Vec::new();
    let desc = description.to_lowercase();

    // Dynamically build suggestions based on registered schemas (D026: Elements Discovery)
    for schema in registry.list_schemas() {
        for (el_type, field_name) in &schema.element_mappings {
            let el_str = el_type.to_string().to_lowercase();
            if el_str.contains(&desc) || field_name.to_lowercase().contains(&desc) {
                suggestions.push(format!(".{}[]", field_name));
                suggestions.push(format!(".{}[] | .name", field_name));
                suggestions.push(format!(
                    ".{}[] | select(.name | contains(\"...\"))",
                    field_name
                ));
            }
        }
    }

    // Sort and dedup suggestions
    suggestions.sort();
    suggestions.dedup();

    if suggestions.is_empty() {
        suggestions = vec![
            ".".to_string(),
            ".[] | keys".to_string(),
            ".[] | select(.name)".to_string(),
        ];
    }

    for (i, suggestion) in suggestions.iter().enumerate() {
        println!("  {}. {}", i + 1, suggestion);
    }

    println!("\nTip: Use 'vecq --explain <query>' to understand what a query does");
}

/// Print available jq filters
fn print_available_filters() {
    let mut filters = std::collections::HashSet::new();

    // Standard library filters
    for (name, _, _) in jaq_std::funs::<jaq_core::data::JustLut<jaq_json::Val>>() {
        filters.insert(name);
    }

    // JSON filters
    for (name, _, _) in jaq_json::funs::<jaq_core::data::JustLut<jaq_json::Val>>() {
        filters.insert(name);
    }

    let mut sorted_filters: Vec<_> = filters.into_iter().collect();
    sorted_filters.sort();

    println!("Available jq filters (jaq engine):");
    println!("===================================");

    // Simple columns for now
    for chunk in sorted_filters.chunks(4) {
        let line = chunk
            .iter()
            .map(|s| format!("{:<20}", s))
            .collect::<Vec<_>>()
            .join("");
        println!("{}", line);
    }

    println!("\nNote: Use 'vecq man' or valid jq documentation for usage details.");
}

/// Format user-friendly error messages
pub fn format_user_error(error: &VecqError) -> String {
    match error {
        VecqError::ParseError {
            file,
            line,
            message,
            ..
        } => {
            format!(
                "Parse error in {} at line {}: {}",
                file.display(),
                line,
                message
            )
        }
        VecqError::QueryError {
            query,
            message,
            suggestion,
        } => {
            let mut msg = format!("Query error in '{}': {}", query, message);
            if let Some(suggestion) = suggestion {
                msg.push_str(&format!("\nSuggestion: {}", suggestion));
            }
            msg
        }
        VecqError::UnsupportedFileType { file_type } => {
            format!(
                "Unsupported file type: {}\nSupported types: {}",
                file_type,
                supported_file_types()
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        VecqError::IoError(e) => {
            format!("File operation failed: {}", e)
        }
        VecqError::ConfigError { message } => {
            format!("Configuration error: {}", message)
        }
        _ => error.to_string(),
    }
}

/// Print detailed explanation of a query
pub fn print_query_explanation(explanation: &vecq::QueryExplanation) {
    println!("Query: {}", explanation.query);
    println!("Description: {}", explanation.description);
    println!("Complexity: {:?}", explanation.complexity);

    if !explanation.operations.is_empty() {
        println!("\nOperations:");
        for (i, op) in explanation.operations.iter().enumerate() {
            println!("  {}. {} - {}", i + 1, op.operation_type, op.description);
            if let Some(ref example) = op.example {
                println!("     Example: {}", example);
            }
        }
    }
}