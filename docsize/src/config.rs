use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use anyhow::{Context, Result};
use std::io::Write;
use crate::ollama::OllamaEngine;

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_model")]
    pub default_model: Option<String>,
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,
    #[serde(default = "default_context_window")]
    pub context_window: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_model: None,
            ollama_url: "http://localhost:11434".to_string(),
            context_window: 4096,
        }
    }
}

fn default_model() -> Option<String> { None }
fn default_ollama_url() -> String { "http://localhost:11434".to_string() }
fn default_context_window() -> usize { 4096 }

#[derive(Serialize, Deserialize, Debug)]
pub struct Session {
    pub history: Vec<Interaction>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Interaction {
    pub query: String,
    pub response: Option<String>,
}

pub fn migrate_config_to_xdg() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let old_dir = home.join(".docsize");
    let new_dir = dirs::config_dir()
        .context("Could not find config directory")?
        .join("docsize");

    if old_dir.exists() && !new_dir.exists() {
        eprintln!("Moving configuration from {} to {}", old_dir.display(), new_dir.display());
        std::fs::rename(&old_dir, &new_dir).context("Failed to move config directory")?;
    } else if !new_dir.exists() {
        std::fs::create_dir_all(&new_dir)?;
    }
    
    Ok(new_dir)
}

pub fn ask_for_url(current: &str) -> Result<String> {
    eprint!("Enter Ollama URL [current: {}]: ", current);
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(current.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

pub async fn resolve_model_interactive(config: &mut Config, update_default: bool) -> Result<(String, bool)> {
    let mut current_url = config.ollama_url.clone();
    
    let (chosen_model, changed) = loop {
        let engine = OllamaEngine { url: current_url.clone() };
        eprintln!("Checking Ollama connection at {}...", current_url);
        
        match engine.list_models().await {
            Ok(models) if !models.is_empty() => {
                use dialoguer::{theme::ColorfulTheme, Select};
                
                let mut items = models.clone();
                items.push("[ Enter custom model name ]".to_string());
                items.push("[ Change Ollama URL ]".to_string());

                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt(format!("Select Ollama model (at {})", current_url))
                    .items(&items)
                    .default(0)
                    .interact()?;

                if selection < models.len() {
                    config.ollama_url = current_url;
                    break (models[selection].clone(), true);
                } else if selection == models.len() {
                    eprint!("Enter model name (e.g., llama3): ");
                    std::io::stderr().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    let trimmed = input.trim().to_string();
                    
                    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                        eprintln!("\n💡 That looks like a URL. Would you like to set it as your Ollama URL and retry?");
                        use dialoguer::Confirm;
                        if Confirm::with_theme(&ColorfulTheme::default()).interact()? {
                            current_url = trimmed;
                            continue;
                        }
                    }
                    
                    if !trimmed.is_empty() {
                        config.ollama_url = current_url;
                        break (trimmed, true);
                    }
                } else {
                    current_url = ask_for_url(&current_url)?;
                }
            }
            _ => {
                eprintln!("\n⚠️  Could not connect to Ollama at {}", current_url);
                eprintln!("1. Retry with a different URL");
                eprintln!("2. Enter a model name manually (skip connection check)");
                
                use dialoguer::{theme::ColorfulTheme, Select};
                let options = vec!["Retry / Change URL", "Enter name manually"];
                let selection = Select::with_theme(&ColorfulTheme::default())
                    .with_prompt("What would you like to do?")
                    .items(&options)
                    .default(0)
                    .interact()?;
                
                if selection == 0 {
                    current_url = ask_for_url(&current_url)?;
                } else {
                    eprint!("Enter model name to use anyway: ");
                    std::io::stderr().flush()?;
                    let mut model_input = String::new();
                    std::io::stdin().read_line(&mut model_input)?;
                    let trimmed = model_input.trim().to_string();

                    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                        eprintln!("\n💡 That looks like a URL. Would you like to set it as your Ollama URL and retry?");
                        use dialoguer::Confirm;
                        if Confirm::with_theme(&ColorfulTheme::default()).interact()? {
                            current_url = trimmed;
                            continue;
                        }
                    }

                    if !trimmed.is_empty() {
                        config.ollama_url = current_url;
                        break (trimmed, true);
                    }
                }
            }
        }
    };
    
    if update_default {
        config.default_model = Some(chosen_model.clone());
    }
    Ok((chosen_model, changed))
}
