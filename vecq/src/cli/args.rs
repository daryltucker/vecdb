// Command-line argument parsing for vecq
// Contains the Args struct, Commands enum, and argument validation logic

use clap::{builder::TypedValueParser, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use vecq::{available_output_formats, supported_file_types, FileType, SchemaRegistry};

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
    # Query functions in a Rust file (use -- to separate query from files)
    vecq src/main.rs -- '.functions[] | select(.visibility == "pub")'

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
pub struct Args {
    /// Input file(s) or directory to process
    #[arg(value_name = "INPUT")]
    pub inputs: Vec<std::path::PathBuf>,

    /// Read jq query from file
    #[arg(short = 'f', long = "from-file")]
    pub from_file: Option<std::path::PathBuf>,

    /// Add directory to search for library modules (functions)
    #[arg(short = 'L', long = "library-path")]
    pub library_path: Vec<std::path::PathBuf>,

    /// jq query to execute on the JSON representation (use -- to separate if multiple inputs)
    /// Can also be specified with -q/--query flag for Unix-style piping
    #[arg(value_name = "QUERY", last = true)]
    pub query_positional: Option<String>,

    /// jq query as a flag (alternative to positional argument)
    #[arg(short = 'q', long = "query")]
    pub query_flag: Option<String>,

    /// Read all inputs into an array before querying (slurp)
    #[arg(short = 's', long)]
    pub slurp: bool,

    /// Convert to JSON without querying
    #[arg(short, long)]
    pub convert: bool,

    /// Enable usage/reference detection in AST parsing
    #[arg(long)]
    pub enable_usages: bool,

    /// Output format
    #[arg(short = 'o', long)]
    #[arg(value_parser = validate_output_format)]
    pub format: Option<String>,

    /// Force JSON output
    #[arg(long, short = 'j', conflicts_with = "format")]
    pub json: bool,

    /// Force Human-Readable output
    #[arg(long, short = 'm', conflicts_with = "format")]
    pub human: bool,

    /// Use grep-compatible output format (filename:line:content)
    #[arg(long)]
    pub grep_format: bool,

    /// Pretty-print JSON output
    #[arg(short, long)]
    pub pretty: bool,

    /// Compact JSON output (no whitespace)
    #[arg(long)]
    pub compact: bool,

    /// Include color in output (auto-detected for terminals)
    #[arg(long)]
    pub color: Option<bool>,

    /// Process files recursively in directories
    #[arg(short = 'R', long)]
    pub recursive: bool,

    /// Limit recursion depth (requires -R)
    #[arg(short = 'd', long)]
    pub depth: Option<usize>,

    /// File type override (auto-detected by default)
    #[arg(short = 't', long)]
    #[arg(value_parser = validate_file_type)]
    pub file_type: Option<FileType>,

    /// Validate query syntax without execution
    #[arg(long)]
    pub validate: bool,

    /// Explain what a query does
    #[arg(long)]
    pub explain: bool,

    /// Show supported file types and exit
    #[arg(long)]
    pub list_types: bool,

    /// Show available output formats and exit
    #[arg(long)]
    pub list_formats: bool,

    /// Verbose output for debugging
    #[arg(short, long)]
    pub verbose: bool,

    /// Number of context lines to include in output
    #[arg(short = 'C', long, default_value = "0")]
    pub context_lines: usize,

    /// Output raw strings, not JSON texts
    #[arg(short = 'r', long = "raw-output")]
    pub raw_output: bool,

    /// Subcommands
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
        input: Option<std::path::PathBuf>,

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
        input: std::path::PathBuf,
    },
    /// List available jq filters and functions
    ListFilters,
    /// List available structural elements (AST nodes)
    Elements,
}

#[derive(Clone)]
pub struct ParseOptions {
    pub file_type: Option<FileType>,
    pub context_lines: usize,
    pub verbose: bool,
    pub recursive: bool,
    pub max_depth: Option<usize>,
    pub enable_usages: bool,
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

pub fn validate_output_format(format: &str) -> Result<String, String> {
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

pub fn validate_file_type(type_str: &str) -> Result<FileType, String> {
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

pub fn get_informed_command() -> clap::Command {
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

pub fn handle_completions(shell: Shell) {
    let mut cmd = get_informed_command();
    generate(shell, &mut cmd, "vecq", &mut std::io::stdout());
}
