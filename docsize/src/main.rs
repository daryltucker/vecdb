use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use std::process::Stdio;
use vecdb_common::OUTPUT;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;

#[derive(Parser)]
#[command(name = "docsize")]
#[command(version = "0.0.9")]
#[command(about = "Contextualized prompt generator for vecdb/vecq")]
struct Args {
    /// The query or prompt to send
    #[arg(index = 1)]
    query: Option<String>,

    /// Target directory (defaults to current directory)
    #[arg(short, long, default_value = ".")]
    dir: PathBuf,

    /// Omit providing directory information
    #[arg(short, long)]
    no_context: bool,

    /// Append to the current conversation session
    #[arg(short, long)]
    append: bool,

    /// Specify the LLM model to use
    /// Specify the LLM model to use (pass no value to select interactively)
    #[arg(short, long, num_args = 0..=1, default_missing_value = "_INTERACTIVE_", require_equals = true)]
    model: Option<String>,

    /// Show the final prompt being sent to the LLM (for debugging)
    #[arg(long)]
    debug: bool,

    /// Enable Smart Routing (detect facets from query)
    #[arg(short, long)]
    smart: bool,

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
    },
}

#[derive(Serialize, Deserialize, Debug)]
struct Config {
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
            context_window: 2048,
        }
    }
}

fn default_model() -> Option<String> { None }
fn default_ollama_url() -> String { "http://localhost:11434".to_string() }
fn default_context_window() -> usize { 2048 }

#[derive(Serialize, Deserialize, Debug)]
struct Session {
    history: Vec<Interaction>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Interaction {
    query: String,
    response: Option<String>,
}

trait InferenceEngine {
    #[allow(dead_code)]
    async fn complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<String>;
    async fn stream_complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<BoxStream<'static, Result<String>>>;
}

struct OllamaEngine {
    url: String,
}

impl OllamaEngine {
    async fn list_models(&self) -> Result<Vec<String>> {
        let client = reqwest::Client::new();
        let api_url = format!("{}/api/tags", self.url.trim_end_matches('/'));
        
        let resp = client.get(&api_url)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;
        let models = json.get("models")
            .and_then(|v| v.as_array())
            .context("Missing 'models' array in Ollama tags output")?;

        let mut names = Vec::new();
        for model in models {
            if let Some(name) = model.get("name").and_then(|v| v.as_str()) {
                names.push(name.to_string());
            }
        }
        Ok(names)
    }
}

impl InferenceEngine for OllamaEngine {
    async fn complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<String> {
        let client = reqwest::Client::new();
        let api_url = format!("{}/api/generate", self.url.trim_end_matches('/'));
        
        let mut payload = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false
        });

        if let Some(opts) = options {
            payload.as_object_mut().unwrap().insert("options".to_string(), opts);
        }

        let resp = client.post(&api_url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;
        let response = json.get("response")
            .and_then(|v| v.as_str())
            .context("Missing 'response' field in Ollama output")?;

        Ok(response.to_string())
    }

    async fn stream_complete(&self, model: &str, prompt: &str, options: Option<serde_json::Value>) -> Result<BoxStream<'static, Result<String>>> {
        let client = reqwest::Client::new();
        let api_url = format!("{}/api/generate", self.url.trim_end_matches('/'));
        
        let mut payload = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": true
        });

        if let Some(opts) = options {
            payload.as_object_mut().unwrap().insert("options".to_string(), opts);
        }

        let resp = client.post(&api_url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;

        let stream = resp.bytes_stream().map(|item| {
            match item {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut combined = String::new();
                    for line in text.lines() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(token) = json.get("response").and_then(|v| v.as_str()) {
                                combined.push_str(token);
                            }
                        }
                    }
                    Ok(combined)
                },
                Err(e) => Err(anyhow::anyhow!(e))
            }
        });

        Ok(stream.boxed())
    }
}

const DEFAULT_PROMPT: &str = r#"
You are a highly capable AI assistant helping with a software project.
Use the provided context to give precise, actionable, and correct answers.

### DIRECTORY STRUCTURE
{{ %DOCSIZE_TREE% }}

### PROJECT OVERVIEW (README)
{{ %DOCSIZE_README% }}

### RELEVANT CODE & SEARCH RESULTS
{{ %DOCSIZE_VECDB_EMBEDDING_RESPONSE% }}

### USER QUERY
{{ %QUERY% }}

Please provide your response in a clear, structured Markdown format. If providing code, ensure it is in fenced blocks with the correct language tag.
"#;

fn estimate_tokens(text: &str) -> usize {
    text.chars().count() / 4
}

fn migrate_config_to_xdg() -> Result<PathBuf> {
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

fn ask_for_url(current: &str) -> Result<String> {
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

async fn resolve_model_interactive(config: &mut Config, update_default: bool) -> Result<(String, bool)> {
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

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging (clean production default)
    vecdb_common::logging::init_logging();

    let args = Args::parse();
    let ctx = &*OUTPUT;

    if let Some(cmd) = args.command {
        match cmd {
            Commands::Man { agent } => {
                if agent {
                    println!("{}", include_str!("../README.md")); // Use actual file
                } else {
                    println!("docsize v0.0.9 - Contextualized LLM wrapper");
                    println!("Usage: docsize [QUERY] [-d DIR] [-m MODEL] [-a]");
                }
                return Ok(());
            }
        }
    }

    let query = if let Some(q) = args.query {
        q
    } else {
        // Prompt for query if missing
        eprint!("Enter your query: ");
        std::io::stderr().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let trimmed = input.trim().to_string();
        if trimmed.is_empty() {
             anyhow::bail!("Query is required.");
        }
        trimmed
    };
    
    // 1. Initialize configuration
    let docsize_dir = migrate_config_to_xdg()?;
    
    let config_path = docsize_dir.join("config.toml");
    let mut config: Config = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        toml::from_str(&content).unwrap_or_default()
    } else {
        Config::default()
    };
    
    let convo_path = docsize_dir.join("convo.json");
    let prompt_path = docsize_dir.join("prompt.md");
    
    if !prompt_path.exists() {
        if std::io::stdin().is_terminal() {
            eprintln!("\n💡 Missing prompt template at {}", prompt_path.display());
            use dialoguer::{theme::ColorfulTheme, Confirm};
            if Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt("Would you like to install the default docsize prompt template?")
                .default(true)
                .interact()? 
            {
                std::fs::write(&prompt_path, DEFAULT_PROMPT)?;
                eprintln!("Default prompt installed.");
            } else {
                anyhow::bail!("Cannot proceed without a prompt template. Please create {}", prompt_path.display());
            }
        } else {
            std::fs::write(&prompt_path, DEFAULT_PROMPT)?;
        }
    }
    
    // 2. Resolve model (Wizard mode if needed)
    let mut save_config = false;
    let mut model = if let Some(m) = args.model {
        if m == "_INTERACTIVE_" {
            // Force interactive selection, do NOT update default
            let (m, s) = resolve_model_interactive(&mut config, false).await?;
            if s { save_config = true; } // save only if URL/other settings changed
            m
        } else {
            m
        }
    } else if let Some(m) = config.default_model.clone() {
        // Sanity check for placeholder strings or URLs in the model field
        if m.starts_with("I_DONT_HAVE_A_DEFAULT") || m.starts_with("http://") || m.starts_with("https://") {
            let (m, s) = resolve_model_interactive(&mut config, true).await?;
            if s { save_config = true; }
            m
        } else {
            m
        }
    } else {
        if std::io::stdin().is_terminal() {
            let (m, s) = resolve_model_interactive(&mut config, true).await?;
            if s { save_config = true; }
            m
        } else {
            anyhow::bail!("No model specified and not interactive. Use -m or set default in config.toml.");
        }
    };

    if save_config {
        let toml_str = toml::to_string_pretty(&config)?;
        std::fs::write(&config_path, toml_str)?;
        eprintln!("Configuration saved to {}", config_path.display());
    }

    let mut session = if args.append && convo_path.exists() {
        let content = std::fs::read_to_string(&convo_path)?;
        serde_json::from_str(&content).unwrap_or(Session { history: vec![] })
    } else {
        Session { history: vec![] }
    };

    // 3. Gather Context
    let mut tree_text = String::new();
    let mut readme_text = String::new();
    let mut search_results = String::new();

    if !args.no_context {
        eprintln!("Gathering context...");
        
        // Tree context via tree -J | vecq
        tree_text = gather_tree_context(&args.dir).await.unwrap_or_else(|e| format!("Tree error: {}", e));

        // Readme context
        readme_text = read_readme(&args.dir).await.unwrap_or_default();

        // Semantic search context via vecdb-server
        search_results = call_vecdb_server(&query, args.debug, args.smart).await.unwrap_or_else(|e| format!("Search error: {}", e));
    }

    // 4. Build Prompt
    let template = std::fs::read_to_string(&prompt_path)?;
    let final_prompt = template
        .replace("{{ %DOCSIZE_TREE% }}", &tree_text)
        .replace("{{ %DOCSIZE_README% }}", &readme_text)
        .replace("{{ %DOCSIZE_VECDB_EMBEDDING_RESPONSE% }}", &search_results)
        .replace("{{ %QUERY% }}", &query);

    if args.debug {
        eprintln!("\n--- DEBUG: FINAL PROMPT ---");
        eprintln!("{}", final_prompt);
        eprintln!("--- END DEBUG ---\n");
    }

    // 5. LLM Interaction
    if ctx.is_interactive { eprintln!("Generating response with {}...", model); }

    let est_tokens = estimate_tokens(&final_prompt);
    let ctx_window = config.context_window;
    let percentage = (est_tokens as f64 / ctx_window as f64) * 100.0;
    
    eprintln!("Context: ~{} / {} tokens ({:.1}%)", est_tokens, ctx_window, percentage);
    if est_tokens > ctx_window {
         eprintln!("⚠️  Warning: Prompt exceeds context window!");
    }
    
    // Output previous history first if append mode is on
    if args.append {
        for interaction in &session.history {
            println!("> {}\n", interaction.query);
            if let Some(res) = &interaction.response {
                println!("% {}\n", res);
            }
        }
    }

    // Print current query marker
    println!("> {}\n", query);
    print!("% ");
    std::io::stdout().flush()?;
    
    let engine = OllamaEngine { url: config.ollama_url.clone() };
    
    let options = serde_json::json!({
        "num_ctx": config.context_window
    });

    let mut stream = match engine.stream_complete(&model, &final_prompt, Some(options.clone())).await {
        Ok(s) => s,
        Err(e) if std::io::stdin().is_terminal() => {
            let err_msg = e.to_string();
            if err_msg.contains("Connection refused") || err_msg.contains("Connect") || err_msg.contains("404") {
               eprintln!("\n⚠️  Ollama connection failed: {}", err_msg);
               eprintln!("It looks like I can't reach Ollama at {}. Would you like to fix the configuration now?", config.ollama_url);
               use dialoguer::{theme::ColorfulTheme, Confirm};
               if Confirm::with_theme(&ColorfulTheme::default()).interact().unwrap_or(false) {
                   let (m, s) = resolve_model_interactive(&mut config, true).await?;
                   if s {
                       let toml_str = toml::to_string_pretty(&config)?;
                       std::fs::write(&config_path, toml_str)?;
                       eprintln!("Configuration updated.");
                   }
                   model = m;
                   let engine = OllamaEngine { url: config.ollama_url.clone() };
                   // Re-print query marker if we retry
                   println!("> {}\n", query);
                   print!("% ");
                   std::io::stdout().flush()?;
                   engine.stream_complete(&model, &final_prompt, Some(options)).await?
               } else {
                   return Err(e);
               }
            } else {
               return Err(e);
            }
        }
        Err(e) => return Err(e),
    };

    let mut full_response = String::new();
    while let Some(chunk) = stream.next().await {
        let token = chunk?;
        print!("{}", token);
        std::io::stdout().flush()?;
        full_response.push_str(&token);
    }
    println!(); // End of response line
    println!();
    println!("---");
    println!(); // Extra spacing

    session.history.push(Interaction {
        query: query.clone(),
        response: Some(full_response),
    });

    let session_json = serde_json::to_string_pretty(&session)?;
    std::fs::write(&convo_path, session_json)?;

    Ok(())
}

async fn gather_tree_context(dir: &Path) -> Result<String> {
    // Cleaner tree: limit depth and ignore common junk
    let output = Command::new("tree")
        .args([
            "-L", "2", 
            "-I", "target|.git|node_modules|dist|build|.cargo",
            &dir.to_string_lossy()
        ])
        .output().await?;

    if !output.status.success() {
        return Ok("".to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn read_readme(dir: &Path) -> Result<String> {
    let mut readme_path = dir.join("README.md");
    if !readme_path.exists() {
        readme_path = dir.join("readme.md");
    }
    if !readme_path.exists() {
        readme_path = dir.join("README.txt"); // Check .txt too
    }
    
    if readme_path.exists() {
        // Read just the first 2KB to avoid blowing context
        let file = std::fs::File::open(readme_path)?;
        let mut handle = file.take(2048);
        let mut buffer = String::new();
        handle.read_to_string(&mut buffer)?;
        if buffer.len() == 2048 {
            buffer.push_str("\n... (truncated) ...");
        }
        Ok(buffer)
    } else {
        Ok("No README found.".to_string())
    }
}

async fn call_vecdb_server(query: &str, debug_mode: bool, smart: bool) -> Result<String> {
    let mut child = Command::new("vecdb-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit()) // Pipe stderr to parent for debugging
        .spawn()
        .context("Failed to spawn vecdb-server")?;

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "docsize", "version": "0.1.0" }
        },
        "id": 1
    });
    stdin.write_all(format!("{}\n", init_req).as_bytes()).await?;
    let _init_resp = reader.next_line().await?.unwrap_or_default();
    
    let search_req = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "name": "search_vectors",
            "arguments": {
                "query": query,
                "smart": smart,
                "json": true
            }
        },
        "id": 2
    });
    stdin.write_all(format!("{}\n", search_req).as_bytes()).await?;
    
    // Read up to 10 lines to find the actual JSON response
    // Add timeout to prevent hangs
    let timeout_duration = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();

    loop {
        if start.elapsed() > timeout_duration {
            if debug_mode { eprintln!("DEBUG: Timed out waiting for vecdb-server response"); }
            break;
        }

        // Use timeout on individual line read
        let line_result = tokio::time::timeout(
            std::time::Duration::from_secs(2), 
            reader.next_line()
        ).await;

        match line_result {
            Ok(Ok(Some(line))) => {
                if debug_mode { eprintln!("DEBUG: Raw Line from Server: {}", line); }
                if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&line) {
                    if resp.get("id").and_then(|id| id.as_i64()) == Some(2) {
                        if let Some(result) = resp.get("result") {
                            if let Some(content_array) = result.get("content") {
                                if let Some(text_val) = content_array[0].get("text") {
                                    let results_text = text_val.as_str().unwrap_or("");
                                    
                                    if results_text.is_empty() || results_text == "[]" {
                                         return Ok("No relevant context found in vector database.".to_string());
                                    }

                                    // Format the JSON results
                                    let results: Vec<serde_json::Value> = serde_json::from_str(results_text).unwrap_or_default();
                                    let mut formatted = String::new();
                                    for (i, res) in results.iter().take(5).enumerate() {
                                        let score = res.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                        let content = res.get("content").and_then(|v| v.as_str()).unwrap_or("");
                                        let path = res.get("metadata").and_then(|m| m.get("path")).and_then(|v| v.as_str()).unwrap_or("unknown");
                                        
                                        formatted.push_str(&format!("--- Result {} (Score: {:.2}, Path: {}) ---\n", i+1, score, path));
                                        formatted.push_str(content);
                                        formatted.push_str("\n\n");
                                    }
                                    
                                    return Ok(formatted);
                                }
                            }
                        }
                    }
                }
            }
            Ok(Ok(None)) | Ok(Err(_)) => break, // EOF or Error
            Err(_) => continue, // Timeout on single line read, keep trying until total timeout
        }
    }

    Ok("No results found".to_string())
}
