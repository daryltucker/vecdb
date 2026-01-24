// CLI command for usage extraction
use anyhow::Result;
use clap::Args;
use std::path::PathBuf;
use vecdb_common::output::OutputFormat;
use vecq::{parse_file_with_options, ElementType};

#[derive(Args, Debug)]
pub struct EnableUsagesArgs {
    /// Source files/directories to analyze
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,

    /// Output format for usage analysis
    #[arg(long, short = 'o', value_enum, default_value = "json")]
    pub output: OutputFormatArg,

    /// Filter usages by type
    #[arg(long, short = 'f', default_value = "all")]
    pub filter: String,

    /// Output format: json, yaml, table, or ast
    #[arg(long, short = 'F', value_enum, default_value = "json")]
    pub format: OutputFormatArg,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum OutputFormatArg {
    Json,
    Yaml,
    Table,
    Ast,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(arg: OutputFormatArg) -> Self {
        match arg {
            OutputFormatArg::Json => OutputFormat::Json,
            OutputFormatArg::Yaml => OutputFormat::Markdown,
            OutputFormatArg::Table => OutputFormat::Text,
            OutputFormatArg::Ast => OutputFormat::Text,
        }
    }
}

pub async fn run(args: EnableUsagesArgs) -> Result<()> {
    let paths = args.paths;
    let filter_type = &args.filter;

    // Process each path
    for path in paths {
        // Detect file type
        let file_type = vecq::detect_file_type(&path.to_string_lossy());

        // Parse with usage detection enabled
        let document = parse_file_with_options(
            &std::fs::read_to_string(&path)?,
            file_type,
            true, // enable_usages
        )
        .await?;

        // Filter usages based on filter type
        let filtered_usages: Vec<_> = match filter_type.as_str() {
            "all" => {
                println!("=== Analysis for: {} ===", path.display());
                document.elements.iter().filter(|e| {
                    matches!(
                        e.element_type,
                        ElementType::FunctionCall
                            | ElementType::VariableReference
                            | ElementType::TypeReference
                            | ElementType::MethodCall
                            | ElementType::Assignment
                            | ElementType::ImportUsage
                    )
                }).collect()
            }
            "calls" => {
                println!("=== Function Calls ===");
                document
                    .elements
                    .iter()
                    .filter(|e| {
                        matches!(
                            e.element_type,
                            ElementType::FunctionCall | ElementType::MethodCall
                        )
                    })
                    .collect()
            }
            "references" => {
                println!("=== Variable References ===");
                document
                    .elements
                    .iter()
                    .filter(|e| matches!(e.element_type, ElementType::VariableReference))
                    .collect()
            }
            "assignments" => {
                println!("=== Assignments ===");
                document
                    .elements
                    .iter()
                    .filter(|e| matches!(e.element_type, ElementType::Assignment))
                    .collect()
            }
            "methods" => {
                println!("=== Method Calls ===");
                document
                    .elements
                    .iter()
                    .filter(|e| matches!(e.element_type, ElementType::MethodCall))
                    .collect()
            }
            _ => {
                println!("=== All Usages ===");
                document
                    .elements
                    .iter()
                    .filter(|e| {
                        matches!(
                            e.element_type,
                            ElementType::FunctionCall
                                | ElementType::VariableReference
                                | ElementType::TypeReference
                                | ElementType::MethodCall
                                | ElementType::Assignment
                                | ElementType::ImportUsage
                        )
                    })
                    .collect()
            }
        };

        // Output the filtered elements
        for usage in &filtered_usages {
            println!("{}", serde_json::to_string_pretty(usage)?);
        }

        // Generate analysis summary
        let total_usages = filtered_usages.len();
        let calls = filtered_usages
            .iter()
            .filter(|e| {
                matches!(
                    e.element_type,
                    ElementType::FunctionCall | ElementType::MethodCall
                )
            })
            .count();
        let references = filtered_usages
            .iter()
            .filter(|e| matches!(e.element_type, ElementType::VariableReference))
            .count();
        let assignments = filtered_usages
            .iter()
            .filter(|e| matches!(e.element_type, ElementType::Assignment))
            .count();

        println!("");
        println!("--- Usage Analysis Summary ---");
        println!("File: {}", path.display());
        println!("Total usages: {}", total_usages);
        println!("Function calls: {}", calls);
        println!("Variable references: {}", references);
        println!("Assignments: {}", assignments);
        println!("");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile;

    #[tokio::test]
    async fn test_enable_usages_basic() -> Result<()> {
        // Create a temporary test file
        let temp_dir = tempfile::tempdir()?;
        let test_file = temp_dir.path().join("test.py");
        fs::write(&test_file, r#"
def hello():
    print("Hello world")
    x = 42
"#)?;

        // Create args for the test
        let args = EnableUsagesArgs {
            paths: vec![test_file],
            output: OutputFormatArg::Json,
            filter: "all".to_string(),
            format: OutputFormatArg::Json,
        };

        // Run the command
        run(args).await
    }
}
