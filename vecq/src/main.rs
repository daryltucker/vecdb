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

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process;
use tokio::fs;
use vecq::{
    detect_file_type, format_results, parse_file, 
    validate_query, explain_query, available_output_formats,
    supported_file_types, FileType, FormatOptions, VecqError, VecqResult,
    UnifiedJsonConverter, JsonConverter, JqQueryEngine, QueryEngine
};
use clap_complete::{generate, Shell};
use clap::CommandFactory;

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
"#)]
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

    /// Output format
    #[arg(short = 'o', long, default_value = "json")]
    #[arg(value_parser = validate_output_format)]
    format: String,

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
}

#[derive(Clone)]
struct ParseOptions {
    file_type: Option<FileType>,
    context_lines: usize,
    verbose: bool,
    recursive: bool,
}

impl From<&Args> for ParseOptions {
    fn from(args: &Args) -> ParseOptions {
        ParseOptions {
            file_type: args.file_type,
            context_lines: args.context_lines,
            verbose: args.verbose,
            recursive: args.recursive,
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
    match type_str.to_lowercase().as_str() {
        "rust" | "rs" => Ok(FileType::Rust),
        "python" | "py" => Ok(FileType::Python),
        "markdown" | "md" => Ok(FileType::Markdown),
        "html" => Ok(FileType::Html),
        "c" => Ok(FileType::C),
        "cpp" | "c++" => Ok(FileType::Cpp),
        "cuda" | "cu" => Ok(FileType::Cuda),
        "go" => Ok(FileType::Go),
        "bash" | "sh" => Ok(FileType::Bash),
        "json" => Ok(FileType::Json),
        _ => Err(format!(
            "Unsupported file type '{}'. Supported types: rust, python, markdown, html, c, cpp, cuda, go, bash, json",
            type_str
        )),
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

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
                let mut cmd = Args::command();
                generate(shell, &mut cmd, "vecq", &mut std::io::stdout());
                return;
            }
            _ => {
                match handle_subcommand(command).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Error: {}", format_user_error(&e));
                        process::exit(1);
                    }
                }
            }
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

async fn run_main_command(mut args: Args) -> VecqResult<()> {
    // Resolve query from file, flag, or positional
    let query_string = if let Some(path) = &args.from_file {
        fs::read_to_string(path).await.map_err(|e| {
            VecqError::IoError(std::io::Error::new(e.kind(), format!("Failed to read query file: {}", e)))
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
    } else {
        args.format.clone()
    };

    // Create format options
    let format_options = FormatOptions {
        pretty_print: args.pretty && !args.compact,
        compact: args.compact,
        grep_compatible: args.grep_format,
        color_output: args.color.unwrap_or_else(|| vecdb_common::OUTPUT.use_color()),
        include_line_numbers: true,
        include_file_paths: true,
        max_width: if output_format == "human" { Some(120) } else { None },
        custom_format: None,
        raw_output: args.raw_output,
    };

    // Resolve inputs: if empty and stdin has data, use stdin
    let mut resolved_inputs = args.inputs.clone();
    if resolved_inputs.is_empty() && vecdb_common::INPUT.has_piped_data {
        resolved_inputs.push(PathBuf::from("-"));
    }

    if resolved_inputs.is_empty() {
        return Err(VecqError::ConfigError {
            message: "No input provided. Usage: vecq <file> [query] or pipe data: cat file | vecq".to_string(),
        });
    }

    if args.slurp {
        let mut all_values = Vec::new();
        for input in resolved_inputs {
            all_values.extend(extract_json_from_input(&input, &ParseOptions::from(&args)).await?);
        }
        let slurp_value = serde_json::Value::Array(all_values);
        process_json_value(slurp_value, &query_string, &engine, &output_format, &format_options).await?;
    } else {
        for input in resolved_inputs {
            let values = extract_json_from_input(&input, &ParseOptions::from(&args)).await?;
            for val in values {
                process_json_value(val, &query_string, &engine, &output_format, &format_options).await?;
            }
        }
    }

    Ok(())
}

/// Extract JSON value(s) from a given input path (file, directory, or stdin)
async fn extract_json_from_input(path: &Path, options: &ParseOptions) -> VecqResult<Vec<serde_json::Value>> {
    if path.to_str() == Some("-") {
        use tokio::io::AsyncReadExt;
        let mut buffer = Vec::new();
        tokio::io::stdin().read_to_end(&mut buffer).await.map_err(|e| {
            VecqError::IoError(std::io::Error::new(e.kind(), format!("Failed to read from stdin: {}", e)))
        })?;
        
        if !FileType::is_likely_text(&buffer) {
             return Err(VecqError::CircuitBreakerTriggered { 
                 message: "Stdin content appears to be binary or malformed text".to_string() 
             });
        }

        let content = String::from_utf8_lossy(&buffer);
        let vals = parse_content_to_json(&content, None, options).await?;
        Ok(vals)
    } else if path.is_file() {
        let vals = parse_file_to_json(path, options).await?;
        Ok(vals)
    } else if path.is_dir() {
        extract_json_from_directory(path, options).await
    } else {
        Err(VecqError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Input path does not exist: {}", path.display()),
        )))
    }
}

async fn extract_json_from_directory(dir_path: &Path, options: &ParseOptions) -> VecqResult<Vec<serde_json::Value>> {
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
                values.extend(Box::pin(extract_json_from_directory(&path, options)).await?);
            }
            continue;
        }

        // Check if file type is supported
        let file_type = options.file_type.unwrap_or_else(|| detect_file_type(path.to_str().unwrap_or("")));
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

async fn parse_file_to_json(path: &Path, options: &ParseOptions) -> VecqResult<Vec<serde_json::Value>> {
    let content_bytes = fs::read(path).await.map_err(|e| {
        VecqError::IoError(std::io::Error::new(
            e.kind(),
            format!("Failed to read file {}: {}", path.display(), e),
        ))
    })?;

    if !FileType::is_likely_text(&content_bytes) {
         return Err(VecqError::CircuitBreakerTriggered { 
             message: format!("File {} appears to be binary or malformed text", path.display()) 
         });
    }

    let content = String::from_utf8_lossy(&content_bytes);
    parse_content_to_json(&content, Some(path), options).await
}

async fn parse_content_to_json(content: &str, path: Option<&Path>, options: &ParseOptions) -> VecqResult<Vec<serde_json::Value>> {
    let file_type = if let Some(p) = path {
        options.file_type.unwrap_or_else(|| detect_file_type(p.to_str().unwrap_or("")))
    } else {
        options.file_type.unwrap_or(FileType::Markdown)
    };

    if file_type == FileType::Unknown {
        return Err(VecqError::UnsupportedFileType {
            file_type: format!("Unknown file type for: {:?}", path),
        });
    }

    let mut json_vals = if file_type == FileType::Json {
        let deserializer = serde_json::Deserializer::from_str(content);
        let mut vals = Vec::new();
        for item in deserializer.into_iter::<serde_json::Value>() {
            let val = item.map_err(|e| VecqError::json_error("Invalid JSON input".to_string(), Some(e)))?;
            vals.push(val);
        }
        vals
    } else {
        let parsed = parse_file(content, file_type).await?;
        let converter = UnifiedJsonConverter::with_default_schemas()
            .with_context_lines(options.context_lines);
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
                    meta_obj.insert("path".to_string(), serde_json::Value::String(path.to_string()));
                }
            } else {
                // Determine if we should inject metadata here? 
                // Mostly we care about injecting into 'attributes' for children
            }

            // Inject into attributes if present, or create it if it looks like a document element
            // We use a heuristic: if it has 'element_type' or 'kind', it's an element
            let is_element = map.contains_key("element_type") || map.contains_key("kind") || map.contains_key("type");
            
            if is_element {
                if let Some(attributes) = map.get_mut("attributes") {
                    if let Some(attr_obj) = attributes.as_object_mut() {
                        attr_obj.insert("file_path".to_string(), serde_json::Value::String(path.to_string()));
                    }
                } else {
                    let mut attr_obj = serde_json::Map::new();
                    attr_obj.insert("file_path".to_string(), serde_json::Value::String(path.to_string()));
                    map.insert("attributes".to_string(), serde_json::Value::Object(attr_obj));
                }
            }

            // check for "metadata" at root level if we are at root
            if !is_element && map.contains_key("elements") {
                 // likely root document
                 if !map.contains_key("metadata") {
                      let mut meta_obj = serde_json::Map::new();
                      meta_obj.insert("path".to_string(), serde_json::Value::String(path.to_string()));
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

async fn process_json_value(
    json_value: serde_json::Value,
    query: &str,
    engine: &JqQueryEngine,
    output_format: &str,
    format_options: &FormatOptions,
) -> VecqResult<()> {
    // Execute query if provided
    let output = if !query.is_empty() {
        let results = engine.execute_query(&json_value, query)?;
        format_results(&results, output_format, format_options)?
    } else {
        // Default: output formatted JSON
        if format_options.pretty_print {
            serde_json::to_string_pretty(&json_value)?
        } else {
            serde_json::to_string(&json_value)?
        }
    };

    // Handle broken pipe gracefully
    use std::io::Write;
    if !output.is_empty() {
        if let Err(e) = writeln!(std::io::stdout(), "{}", output) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        return Err(VecqError::IoError(e));
        }
    }

    Ok(())
}


async fn handle_subcommand(command: Commands) -> VecqResult<()> {
    match command {
        Commands::Doc { input } => {
            let options = ParseOptions {
                file_type: None,
                context_lines: 0,
                verbose: false,
                recursive: false,
            };
            let values = extract_json_from_input(&input, &options).await?;
            let engine = JqQueryEngine::new_hermetic();
            
            // For now, simpler than full iterator: just process each file
            for val in values {
                let result = engine.execute_query(&val, "markdown")?;
                 // If the result is a string, print it raw. JSON otherwise.
                match result {
                    serde_json::Value::String(s) => println!("{}", s),
                    _ => println!("{}", result),
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
        Commands::Completions { .. } => unreachable!("Handled in main"),
        Commands::Syntax { input, language } => {
            use syntect::easy::HighlightFile;
            use syntect::parsing::SyntaxSet;
            use syntect::highlighting::{ThemeSet, Style};
            use syntect::util::as_24_bit_terminal_escaped;
            use std::io::BufRead;

            let ss = SyntaxSet::load_defaults_newlines();
            let ts = ThemeSet::load_defaults();
            
            // Use a dark theme by default, or fallback
            // let theme = &ts.themes["base16-ocean.dark"];
            let theme = ts.themes.get("base16-ocean.dark").unwrap_or_else(|| ts.themes.values().next().unwrap());

            let manual_syntax = if let Some(lang) = language {
                 ss.find_syntax_by_token(&lang).or_else(|| ss.find_syntax_by_extension(&lang))
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
                         let regions: Vec<(Style, &str)> = h.highlight_line(&line, &ss).map_err(|e| VecqError::ConfigError{message: e.to_string()})?;
                         print!("{}", as_24_bit_terminal_escaped(&regions[..], true));
                         line.clear();
                    }

                } else {
                     let mut highlighter = HighlightFile::new(path, &ss, theme).map_err(|e| VecqError::IoError(std::io::Error::other(e)))?;
                     let mut line = String::new();
                     while highlighter.reader.read_line(&mut line)? > 0 {
                         let regions: Vec<(Style, &str)> = highlighter.highlight_lines.highlight_line(&line, &ss).map_err(|e| VecqError::ConfigError{message: e.to_string()})?;
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
                let syntax = manual_syntax.or_else(|| ss.find_syntax_by_first_line(&content)).unwrap_or_else(|| ss.find_syntax_plain_text());
                let mut h = syntect::easy::HighlightLines::new(syntax, theme);
                
                for line in content.lines() {
                     let regions: Vec<(Style, &str)> = h.highlight_line(line, &ss).map_err(|e| VecqError::ConfigError{message: e.to_string()})?;
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
    
    // Core filters
    for (name, _, _) in jaq_core::core() {
        filters.insert(name);
    }
    
    // Standard library filters
    for def in jaq_std::std() {
        filters.insert(def.lhs.name);
    }
    
    let mut sorted_filters: Vec<_> = filters.into_iter().collect();
    sorted_filters.sort();
    
    println!("Available jq filters (jaq engine):");
    println!("===================================");
    
    // Group roughly by starting letter for readability, or just columns
    // Simple columns for now
    for chunk in sorted_filters.chunks(4) {
        let line = chunk.iter().map(|s| format!("{:<20}", s)).collect::<Vec<String>>().join("");
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
    
    // Simple pattern matching for common requests
    let suggestions = match description.to_lowercase().as_str() {
        desc if desc.contains("function") => vec![
            ".functions[]",
            ".functions[] | .name",
            ".functions[] | select(.visibility == \"pub\")",
        ],
        desc if desc.contains("struct") => vec![
            ".structs[]",
            ".structs[] | .name",
            ".structs[] | select(.name | contains(\"Test\"))",
        ],
        desc if desc.contains("header") => vec![
            ".headers[]",
            ".headers[] | select(.level == 2)",
            ".headers[] | .title",
        ],
        _ => vec![
            ".",
            ".[] | keys",
            ".[] | select(.name)",
        ],
    };

    for (i, suggestion) in suggestions.iter().enumerate() {
        println!("  {}. {}", i + 1, suggestion);
    }
    
    println!("\nTip: Use 'vecq --explain <query>' to understand what a query does");
}

fn format_user_error(error: &VecqError) -> String {
    match error {
        VecqError::ParseError { file, line, message, .. } => {
            format!("Parse error in {} at line {}: {}", file.display(), line, message)
        }
        VecqError::QueryError { query, message, suggestion } => {
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