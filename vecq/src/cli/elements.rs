// Elements subcommand handler for vecq CLI
// Handles the 'vecq elements' command and its subcommands

use clap::ArgMatches;
use vecq::{SchemaRegistry, VecqError, VecqResult};

pub async fn handle_elements_command(matches: &ArgMatches) -> VecqResult<()> {
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
            super::args::validate_file_type(lang_name).map_err(|e| VecqError::ConfigError { message: e })?;
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