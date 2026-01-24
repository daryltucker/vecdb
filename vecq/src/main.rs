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

use clap::builder::TypedValueParser;
use clap::{ArgMatches, CommandFactory, FromArgMatches};
use clap::{Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::path::{Path, PathBuf};
use std::process;
use tokio::fs;
use vecq::{
    available_output_formats, detect_file_type, explain_query, format_results, parse_file,
    parse_file_with_options, supported_file_types, validate_query, FileType, FormatOptions,
    JqQueryEngine, JsonConverter, QueryEngine, SchemaRegistry, UnifiedJsonConverter, VecqError,
    VecqResult,
};

mod man_cmd;

/// vecq - jq for source code
///
/// Convert any structured document to queryable JSON and query with jq syntax.
/// Supports Rust, Python, Markdown, C/C++, CUDA, Go, and Bash files.
#[derive(Parser)]
#[command(name = "vecq")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "jq for source code - convert documents to queryable JSON")]
#[command(long_about = r#"
vecq converts any structured document (source code, markdown, etc.) into queryable JSON
and enables jq-like querying with natural language support.

EXAMPLES:
    # Query functions in a Rust file
    vecq src/main.rs '.functions[] | select(.visibility == "pub")'
    
    # Convert file to JSON without querying
    vecq --convert README.md
    
    # Batch process with grep-compatible output
    vecq src/ '.functions[]' --grep-format | grep "TODO"
    
    # Human-readable table output
    vecq src/lib.rs '.structs[]' --format human
    
    # Pipeline with Unix tools
    find . -name "*.rs" | xargs vecq --grep-format | grep "pub fn"

SUPPORTED FILE TYPES:
    Rust (.rs), Python (.py), Markdown (.md), C (.c, .h), 
    C++ (.cpp, .hpp), CUDA (.cu), Go (.go), Bash (.sh)

OUTPUT FORMATS:
    json (default), grep, human

AGENT USAGE:
    Run `vecq man --agent` for the definitive Agent Interface Manual.
"#)]
#[command(after_help = "See `vecq man --agent` for Agent Interface documentation.")]
struct Args {
    /// Input file(s) or directory to process
    #[arg(value_name = "INPUT")]
    inputs: Vec<PathBuf>,

    /// Read jq query from file
    #[arg(short = 'f', long = "from-file")]
    from_file: Option<PathBuf>,

    /// Add directory to search for library modules (functions)
    #[arg(short = 'L', long = "library-path")]
    library_path: Vec<PathBuf>,

    /// jq query to execute on the JSON representation (use -- to separate if multiple inputs)
    /// Can also be specified with -q/--query flag for Unix-style piping
    #[arg(value_name = "QUERY", last = true)]
    query_positional: Option<String>,

    /// jq query as a flag (alternative to positional argument)
    #[arg(short = 'q', long = "query")]
    query_flag: Option<String>,

    /// Read all inputs into an array before querying (slurp)
    #[arg(short = 's', long)]
    slurp: bool,

    /// Convert to JSON without querying
    #[arg(short, long)]
    convert: bool,

    /// Enable usage/reference detection in AST parsing
    #[arg(long)]
    enable_usages: bool,

    /// Output format
    #[arg(short = 'o', long)]
    #[arg(value_parser = validate_output_format)]
    format: Option<String>,

    /// Force JSON output
    #[arg(long, short = 'j', conflicts_with = "format")]
    json: bool,

    /// Force Human-Readable output
    #[arg(long, short = 'm', conflicts_with = "format")]
    human: bool,

    /// Use grep-compatible output format (filename:line:content)
    #[arg(long)]
    grep_format: bool,

    /// Pretty-print JSON output
    #[arg(short, long)]
    pretty: bool,

    /// Compact JSON output (no whitespace)
    #[arg(long)]
    compact: bool,

    /// Include color in output (auto-detected for terminals)
    #[arg(long)]
    color: Option<bool>,

    /// Process files recursively in directories
    #[arg(short = 'R', long)]
    recursive: bool,

    /// Limit recursion depth (requires -R)
    #[arg(short = 'd', long)]
    depth: Option<usize>,

    /// File type override (auto-detected by default)
    #[arg(short = 't', long)]
    #[arg(value_parser = validate_file_type)]
    file_type: Option<FileType>,

    /// Validate query syntax without execution
    #[arg(long)]
    validate: bool,

    /// Explain what a query does
    #[arg(long)]
    explain: bool,

    /// Show supported file types and exit
    #[arg(long)]
    list_types: bool,

    /// Show available output formats and exit
    #[arg(long)]
    list_formats: bool,

    /// Verbose output for debugging
    #[arg(short, long)]
    verbose: bool,

    /// Number of context lines to include in output
    #[arg(short = 'C', long, default_value = "0")]
    context_lines: usize,

    /// Output raw strings, not JSON texts
    #[arg(short = 'r', long = "raw-output")]
    raw_output: bool,

    /// Subcommands
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show manual page
    Man {
        /// Show agent-optimized documentation
        #[arg(long)]
        agent: bool,

        /// Specific command to view
        #[arg(index = 1)]
        command: Option<String>,
    },
    /// Suggest jq queries for natural language input
    Suggest {
        /// Natural language query description
        description: String,
    },
    /// Syntax highlight a file
    Syntax {
        /// Input file (optional, defaults to stdin)
        #[arg(value_name = "INPUT")]
        input: Option<PathBuf>,

        /// Force language syntax (e.g., 'md', 'rs', 'json')
        #[arg(short = 'l', long)]
        language: Option<String>,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate documentation from source code
    Doc {
        /// Input file
        #[arg(value_name = "INPUT")]
        input: PathBuf,
    },
    /// List available jq filters and functions
    ListFilters,
    /// List available structural elements (AST nodes)
    Elements,
}

#[derive(Clone)]
struct ParseOptions {
    file_type: Option<FileType>,
    context_lines: usize,
    verbose: bool,
    recursive: bool,
    max_depth: Option<usize>,
    enable_usages: bool,
}

impl From<&Args> for ParseOptions {
    fn from(args: &Args) -> ParseOptions {
        ParseOptions {
            file_type: args.file_type,
            context_lines: args.context_lines,
            verbose: args.verbose,
            recursive: args.recursive,
            max_depth: args.depth,
            enable_usages: args.enable_usages,
        }
    }
}

fn validate_output_format(format: &str) -> Result<String, String> {
    let available = available_output_formats();
    if available.contains(&format.to_string()) {
        Ok(format.to_string())
    } else {
        Err(format!(
            "Invalid format '{}'. Available formats: {}",
            format,
            available.join(", ")
        ))
    }
}

fn validate_file_type(type_str: &str) -> Result<FileType, String> {
    let type_str = type_str.to_lowercase();
    for ft in supported_file_types() {
        if ft.to_string().to_lowercase() == type_str
            || ft.file_extensions().iter().any(|&ext| ext == type_str)
        {
            return Ok(ft);
        }
    }

    // Special handlings for non-standard but common aliases
    match type_str.as_str() {
        "c++" => return Ok(FileType::Cpp),
        "txt" => return Ok(FileType::Text),
        _ => {}
    }

    Err(format!(
        "Unsupported file type '{}'. Supported types: {}",
        type_str,
        supported_file_types()
            .iter()
            .map(|ft| ft.to_string().to_lowercase())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn get_informed_command() -> clap::Command {
    let mut cmd = Args::command();

    // Dynamically discover supported file types for suggestions/help
    let types: Vec<_> = supported_file_types()
        .iter()
        .map(|ft| ft.to_string().to_lowercase())
        .collect::<Vec<_>>();

    // Standard extensions for help text
    let mut ext_types = types.clone();
    ext_types.extend(vec![
        "rs".to_string(),
        "py".to_string(),
        "md".to_string(),
        "sh".to_string(),
    ]);
    ext_types.sort();
    ext_types.dedup();

    // 1. Inject possible values into --file-type / -t
    let static_ext_types: Vec<&'static str> = ext_types
        .into_iter()
        .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
        .collect();

    let ft_parser = clap::builder::PossibleValuesParser::new(static_ext_types.clone())
        .map(|s: String| validate_file_type(&s).unwrap());

    cmd = cmd.mut_arg("file_type", |arg| arg.value_parser(ft_parser.clone()));

    // 2. Inject into 'elements' subcommand
    cmd = cmd.mut_subcommand("elements", |sub| {
        let mut sub = sub
            .about("List available structural elements by language")
            .arg(
                clap::Arg::new("json")
                    .long("json")
                    .help("Output as JSON")
                    .action(clap::ArgAction::SetTrue),
            );

        let registry = SchemaRegistry::new();
        for ft in supported_file_types() {
            let ft_name: &'static str = Box::leak(ft.to_string().to_lowercase().into_boxed_str());
            let mut lang_sub = clap::Command::new(ft_name).about(format!("Browse {} elements", ft));

            // Add aliases from extensions
            for ext_ref in ft.file_extensions() {
                let ext: &str = ext_ref;
                if ext != ft_name {
                    lang_sub =
                        lang_sub.alias(Box::leak(ext.to_string().into_boxed_str()) as &'static str);
                }
            }
            // Manual overrides removed as they are now handled by extensions

            if let Ok(schema) = registry.get_schema(ft) {
                let mut elements: Vec<String> = schema.element_mappings.values().cloned().collect();
                elements.extend(schema.required_fields.clone());
                elements.sort();
                elements.dedup();

                let static_el: Vec<&'static str> = elements
                    .into_iter()
                    .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
                    .collect();

                lang_sub = lang_sub
                    .arg(
                        clap::Arg::new("element")
                            .value_parser(clap::builder::PossibleValuesParser::new(static_el))
                            .help("Specific structural element to filter by"),
                    )
                    .arg(
                        clap::Arg::new("json")
                            .long("json")
                            .help("Output as JSON")
                            .action(clap::ArgAction::SetTrue),
                    );
            }
            sub = sub.subcommand(lang_sub);
        }
        sub
    });

    cmd
}

#[tokio::main]
async fn main() {
    let cmd = get_informed_command();
    let matches = cmd.get_matches();
    let args = Args::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    // Handle informational commands first
    if args.list_types {
        print_supported_types();
        return;
    }

    if args.list_formats {
        print_available_formats();
        return;
    }

    // Handle subcommands
    if let Some(command) = args.command {
        match command {
            Commands::Completions { shell } => {
                let mut cmd = get_informed_command();
                generate(shell, &mut cmd, "vecq", &mut std::io::stdout());
                return;
            }
            Commands::Elements => {
                if let Err(e) = handle_elements_command(&matches).await {
                    eprintln!("Error: {}", format_user_error(&e));
                    process::exit(1);
                }
            }
            _ => match handle_subcommand(command).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("Error: {}", format_user_error(&e));
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
            match validate_query(query) {
                Ok(()) => {
                    println!("Query syntax is valid");
                    return;
                }
                Err(e) => {
                    eprintln!("Query validation failed: {}", format_user_error(&e));
                    process::exit(1);
                }
            }
        }

        if args.explain {
            match explain_query(query) {
                Ok(explanation) => {
                    print_query_explanation(&explanation);
                    return;
                }
                Err(e) => {
                    eprintln!("Query explanation failed: {}", format_user_error(&e));
                    process::exit(1);
                }
            }
        }
    }

    // Main processing
    match run_main_command(args).await {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {}", format_user_error(&e));
            process::exit(1);
        }
    }
}

async fn handle_elements_command(matches: &ArgMatches) -> VecqResult<()> {
    let el_matches =
        matches
            .subcommand_matches("elements")
            .ok_or_else(|| VecqError::ConfigError {
                message: "Elements subcommand not found".to_string(),
            })?;
    let mut json = el_matches.get_flag("json");
    let registry = SchemaRegistry::new();

    if let Some((lang_name, sub_matches)) = el_matches.subcommand() {
        if sub_matches.get_flag("json") {
            json = true;
        }
        let ft =
            validate_file_type(lang_name).map_err(|e| VecqError::ConfigError { message: e })?;
        let schema = registry.get_schema(ft)?;
        let element = sub_matches.get_one::<String>("element");

        if let Some(target_field) = element {
            // Drill down into a specific element's attributes
            // First, find the ElementType that maps to this field name
            let element_type = schema
                .element_mappings
                .iter()
                .find(|(_, field)| *field == target_field)
                .map(|(et, _)| *et);

            let attributes = if let Some(et) = element_type {
                schema.get_attributes(et)
            } else {
                Vec::new()
            };

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&attributes).map_err(
                        |e| VecqError::json_error(
                            "Failed to serialize attributes".to_string(),
                            Some(e)
                        )
                    )?
                );
            } else {
                println!("Attributes for {} {}:", ft, target_field);
                if attributes.is_empty() {
                    println!("  (none or no specific metadata registered)");
                } else {
                    for attr in attributes {
                        println!("  - {}", attr);
                    }
                }
            }
        } else {
            // List structural elements for the language
            let mut elements: Vec<String> = schema.element_mappings.values().cloned().collect();
            elements.extend(schema.required_fields.clone());
            elements.sort();
            elements.dedup();

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&elements).map_err(|e| VecqError::json_error(
                        "Failed to serialize elements".to_string(),
                        Some(e)
                    ))?
                );
            } else {
                println!("Structural elements for {}:", ft);
                if elements.is_empty() {
                    println!("  (none found)");
                } else {
                    for chunk in elements.chunks(4) {
                        let line = chunk
                            .iter()
                            .map(|s| format!("{:<20}", s))
                            .collect::<Vec<String>>()
                            .join("");
                        println!("  {}", line);
                    }
                }
            }
        }
    } else {
        // Root 'elements' command: List supported languages by default (less verbose)
        let schemas = registry.list_schemas();

        if json {
            // Maintain full verbosity for Agents/automated tools
            let mut result = serde_json::Map::new();
            for schema in schemas {
                let mut elements: Vec<String> = schema.element_mappings.values().cloned().collect();
                elements.extend(schema.required_fields.clone());
                elements.sort();
                elements.dedup();
                result.insert(
                    schema.file_type.to_string(),
                    serde_json::Value::Array(
                        elements
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&result).map_err(|e| VecqError::json_error(
                    "Failed to serialize schemas".to_string(),
                    Some(e)
                ))?
            );
        } else {
            println!("Supported languages for structural extraction:");
            println!(
                "(Use 'vecq elements <lang>' to see structural elements for a specific language)"
            );
            let mut languages: Vec<String> =
                schemas.iter().map(|s| s.file_type.to_string()).collect();
            languages.sort();

            for chunk in languages.chunks(4) {
                let line = chunk
                    .iter()
                    .map(|s| format!("{:<20}", s))
                    .collect::<Vec<String>>()
                    .join("");
                println!("  {}", line);
            }
        }
    }
    Ok(())
}

async fn run_main_command(mut args: Args) -> VecqResult<()> {
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

    use std::path::PathBuf;

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
        use vecdb_common::output::{OutputContext, OutputFormat};
        // Detect fresh context to check stdout TTY status
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
            all_values.extend(extract_json_from_input(&input, &ParseOptions::from(&args)).await?);
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
            let values = extract_json_from_input(&input, &ParseOptions::from(&args)).await?;
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

/// Extract JSON value(s) from a given input path (file, directory, or stdin)
async fn extract_json_from_input(
    path: &Path,
    options: &ParseOptions,
) -> VecqResult<Vec<serde_json::Value>> {
    if path.to_str() == Some("-") {
        use tokio::io::AsyncReadExt;
        let mut buffer = Vec::new();
        tokio::io::stdin()
            .read_to_end(&mut buffer)
            .await
            .map_err(|e| {
                VecqError::IoError(std::io::Error::new(
                    e.kind(),
                    format!("Failed to read from stdin: {}", e),
                ))
            })?;

        if !FileType::is_likely_text(&buffer) {
            return Err(VecqError::CircuitBreakerTriggered {
                message: "Stdin content appears to be binary or malformed text".to_string(),
            });
        }

        let content = String::from_utf8_lossy(&buffer);
        let vals = parse_content_to_json(&content, None, options).await?;
        Ok(vals)
    } else if path.is_file() {
        let vals = parse_file_to_json(path, options).await?;
        Ok(vals)
    } else if path.is_dir() {
        extract_json_from_directory(path, options, 0).await
    } else {
        Err(VecqError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Input path does not exist: {}", path.display()),
        )))
    }
}

async fn extract_json_from_directory(
    dir_path: &Path,
    options: &ParseOptions,
    current_depth: usize,
) -> VecqResult<Vec<serde_json::Value>> {
    let mut entries = fs::read_dir(dir_path).await.map_err(|e| {
        VecqError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to read directory {}: {}", dir_path.display(), e),
        ))
    })?;

    let mut values = Vec::new();

    while let Some(entry) = entries.next_entry().await.map_err(VecqError::IoError)? {
        let path = entry.path();

        if path.is_dir() {
            if options.recursive {
                if let Some(max) = options.max_depth {
                    if current_depth >= max {
                        continue;
                    }
                }
                values.extend(
                    Box::pin(extract_json_from_directory(
                        &path,
                        options,
                        current_depth + 1,
                    ))
                    .await?,
                );
            }
            continue;
        }

        // Check if file type is supported
        let file_type = options
            .file_type
            .unwrap_or_else(|| detect_file_type(path.to_str().unwrap_or("")));
        if file_type == FileType::Unknown {
            continue;
        }

        match parse_file_to_json(&path, options).await {
            Ok(vals) => values.extend(vals),
            Err(_) if !options.verbose => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(values)
}

async fn parse_file_to_json(
    path: &Path,
    options: &ParseOptions,
) -> VecqResult<Vec<serde_json::Value>> {
    let content_bytes = fs::read(path).await.map_err(|e| {
        VecqError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to read file {}: {}", path.display(), e),
        ))
    })?;

    if !FileType::is_likely_text(&content_bytes) {
        return Err(VecqError::CircuitBreakerTriggered {
            message: format!(
                "File {} appears to be binary or malformed text",
                path.display()
            ),
        });
    }

    let content = String::from_utf8_lossy(&content_bytes);
    parse_content_to_json(&content, Some(path), options).await
}

async fn parse_content_to_json(
    content: &str,
    path: Option<&Path>,
    options: &ParseOptions,
) -> VecqResult<Vec<serde_json::Value>> {
    let file_type = if let Some(p) = path {
        options
            .file_type
            .unwrap_or_else(|| detect_file_type(p.to_str().unwrap_or("")))
    } else {
        options.file_type.unwrap_or(FileType::Unknown)
    };

    let mut json_vals = if file_type == FileType::Text {
        // Treat content as raw string, wrapped in single JSON string
        vec![serde_json::Value::String(content.to_string())]
    } else if file_type == FileType::Json
        || (file_type == FileType::Unknown
            && (content.trim_start().starts_with('{') || content.trim_start().starts_with('[')))
    {
        let deserializer = serde_json::Deserializer::from_str(content);
        let mut vals = Vec::new();
        for item in deserializer.into_iter::<serde_json::Value>() {
            match item {
                Ok(val) => vals.push(val),
                Err(e) => {
                    if file_type == FileType::Json {
                        return Err(VecqError::json_error(
                            "Invalid JSON input".to_string(),
                            Some(e),
                        ));
                    } else {
                        return Err(VecqError::UnsupportedFileType {
                            file_type: "Unknown (failed JSON heuristic)".to_string(),
                        });
                    }
                }
            }
        }
        vals
    } else {
        if file_type == FileType::Unknown {
            return Err(VecqError::UnsupportedFileType {
                file_type: format!("Unknown file type for: {:?}", path),
            });
        }
        let parsed = if options.enable_usages {
            parse_file_with_options(content, file_type, true).await?
        } else {
            parse_file(content, file_type).await?
        };
        let converter =
            UnifiedJsonConverter::with_default_schemas().with_context_lines(options.context_lines);
        vec![converter.convert(parsed)?]
    };

    // Inject Path into Metadata and recursively into all object nodes
    if let Some(p) = path {
        let path_str = p.to_string_lossy().to_string();
        for val in &mut json_vals {
            inject_file_path_recursive(val, &path_str);
        }
    }

    Ok(json_vals)
}

fn inject_file_path_recursive(value: &mut serde_json::Value, path: &str) {
    match value {
        serde_json::Value::Object(map) => {
            // Inject into metadata if present (root node usually)
            if let Some(metadata) = map.get_mut("metadata") {
                if let Some(meta_obj) = metadata.as_object_mut() {
                    meta_obj.insert(
                        "path".to_string(),
                        serde_json::Value::String(path.to_string()),
                    );
                }
            }

            // Inject into attributes if present, or create it if it looks like a document element
            let is_element = map.contains_key("element_type")
                || map.contains_key("kind")
                || map.contains_key("type");

            if is_element {
                if let Some(attributes) = map.get_mut("attributes") {
                    if let Some(attr_obj) = attributes.as_object_mut() {
                        attr_obj.insert(
                            "file_path".to_string(),
                            serde_json::Value::String(path.to_string()),
                        );
                    }
                } else {
                    let mut attr_obj = serde_json::Map::new();
                    attr_obj.insert(
                        "file_path".to_string(),
                        serde_json::Value::String(path.to_string()),
                    );
                    map.insert(
                        "attributes".to_string(),
                        serde_json::Value::Object(attr_obj),
                    );
                }
            }

            // check for "metadata" at root level if we are at root
            if !is_element && map.contains_key("elements") {
                // likely root document
                if !map.contains_key("metadata") {
                    let mut meta_obj = serde_json::Map::new();
                    meta_obj.insert(
                        "path".to_string(),
                        serde_json::Value::String(path.to_string()),
                    );
                    map.insert("metadata".to_string(), serde_json::Value::Object(meta_obj));
                }
            }

            // Recurse into all fields
            for (_, v) in map.iter_mut() {
                inject_file_path_recursive(v, path);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                inject_file_path_recursive(v, path);
            }
        }
        _ => {}
    }
}

// Synchronous version to support StdoutLock
fn process_json_value(
    json_value: serde_json::Value,
    query: &str,
    engine: &JqQueryEngine,
    output_format: &str,
    format_options: &FormatOptions,
    writer: &mut impl std::io::Write,
) -> VecqResult<()> {
    // Execute query if provided
    if !query.is_empty() {
        let results = engine.execute_query(&json_value, query)?;

        for result in results {
            let output = format_results(&result, output_format, format_options)?;
            if !output.is_empty() {
                if let Err(e) = writeln!(writer, "{}", output) {
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        std::process::exit(0);
                    }
                    return Err(VecqError::IoError(e));
                }
            }
        }
    } else {
        // Default: output formatted JSON
        let output = if format_options.pretty_print {
            serde_json::to_string_pretty(&json_value)?
        } else {
            serde_json::to_string(&json_value)?
        };

        if let Err(e) = writeln!(writer, "{}", output) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                std::process::exit(0);
            }
            return Err(VecqError::IoError(e));
        }
    };

    Ok(())
}

// ...
// Actually, I can't return Err inside the prompt string haha.
// I will abort this tool call and issue a correct one.
// I will just execute NOTHING in the replacement content to trigger a "no change" or just use multi_replace in next turn.
// Wait, I can't abort comfortably. I will synthesize the replacement for just `run_main_command` first?
// No, I need to match the signature. If I change one, I break the build until I change the other.

async fn handle_subcommand(command: Commands) -> VecqResult<()> {
    match command {
        Commands::Doc { input } => {
            let options = ParseOptions {
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
            use std::io::BufRead;
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
                use std::io::Read;
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

fn print_supported_types() {
    println!("Supported file types:");
    for file_type in supported_file_types() {
        let extensions = file_type.file_extensions().join(", ");
        println!("  {} ({})", file_type, extensions);
    }
}

fn print_available_formats() {
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

fn print_query_explanation(explanation: &vecq::QueryExplanation) {
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

fn print_query_suggestions(description: &str) {
    println!("Query suggestions for: \"{}\"", description);

    let registry = SchemaRegistry::new();
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

fn format_user_error(error: &VecqError) -> String {
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

// TTY detection is now handled by vecdb_common::OUTPUT
// See: docs/planning/PHILOSOPHY.md and .agent/rules/OUTPUT.md

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_validation() {
        assert!(validate_output_format("json").is_ok());
        assert!(validate_output_format("grep").is_ok());
        assert!(validate_output_format("human").is_ok());
        assert!(validate_output_format("invalid").is_err());
    }

    #[test]
    fn test_file_type_validation() {
        assert_eq!(validate_file_type("rust").unwrap(), FileType::Rust);
        assert_eq!(validate_file_type("python").unwrap(), FileType::Python);
        assert_eq!(validate_file_type("markdown").unwrap(), FileType::Markdown);
        assert!(validate_file_type("invalid").is_err());
    }

    #[test]
    fn test_error_formatting() {
        let error = VecqError::UnsupportedFileType {
            file_type: "unknown".to_string(),
        };
        let formatted = format_user_error(&error);
        assert!(formatted.contains("Unsupported file type"));
        assert!(formatted.contains("Supported types"));
    }
}
