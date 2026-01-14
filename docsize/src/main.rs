mod args;
mod config;
mod ollama;
mod context;

use anyhow::Result;
use clap::Parser;
use std::io::Write;
use std::io::IsTerminal;
use vecdb_common::OUTPUT;
use futures_util::StreamExt;

// Re-exports/Use from modules
use crate::args::{Args, Commands};
use crate::config::{Config, Session, Interaction, migrate_config_to_xdg, resolve_model_interactive};
use crate::ollama::InferenceEngine;
use crate::context::{gather_tree_context, read_readme, call_vecdb_server};

const DEFAULT_PROMPT: &str = r#"
You are a highly capable AI assistant helping with a software project.
Use the provided context to give precise, actionable, and correct answers.

### PROJECT CONTEXT
Dominant Language: {{ %PROJECT_LANG% }}

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
                    println!("{}", include_str!("../README.md")); 
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
            let (m, s) = resolve_model_interactive(&mut config, false).await?;
            if s { save_config = true; } 
            m
        } else {
            m
        }
    } else if let Some(m) = config.default_model.clone() {
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
    let mut project_lang = String::new();
    let mut readme_text = String::new();
    let mut search_results = String::new();

    if !args.no_context {
        eprintln!("Gathering context...");
        
        // Tree context
        let (tree, lang) = gather_tree_context(&args.dir).await.unwrap_or_else(|e| (format!("Tree error: {}", e), "Unknown".to_string()));
        tree_text = tree;
        project_lang = lang;

        // Readme context
        readme_text = read_readme(&args.dir).await.unwrap_or_default();

        // Semantic search context with sanitization
        search_results = call_vecdb_server(&query, &args.dir, args.debug, args.smart).await.unwrap_or_else(|e| format!("Search error: {}", e));
    }

    // 4. Build Prompt
    let template = std::fs::read_to_string(&prompt_path)?;
    let final_prompt = template
        .replace("{{ %DOCSIZE_TREE% }}", &tree_text)
        .replace("{{ %DOCSIZE_README% }}", &readme_text)
        .replace("{{ %DOCSIZE_VECDB_EMBEDDING_RESPONSE% }}", &search_results)
        .replace("{{ %QUERY% }}", &query)
        .replace("{{ %PROJECT_LANG% }}", &project_lang);


    if args.debug {
        eprintln!("\n--- DEBUG: PROMPT CONTEXT BREAKDOWN (Estimated) ---");
        eprintln!("Tree Context:    {:>5} tokens (len: {})", estimate_tokens(&tree_text), tree_text.len());
        eprintln!("README Context:  {:>5} tokens (len: {})", estimate_tokens(&readme_text), readme_text.len());
        eprintln!("Search Context:  {:>5} tokens (len: {})", estimate_tokens(&search_results), search_results.len());
        eprintln!("User Query:      {:>5} tokens (len: {})", estimate_tokens(&query), query.len());
        eprintln!("---------------------------------------");
        eprintln!("TOTAL PROMPT:    {:>5} tokens (len: {})", estimate_tokens(&final_prompt), final_prompt.len());
        eprintln!("---------------------------------------");

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
    
    // Create engine using the module logic
    use crate::ollama::OllamaEngine;
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
    println!(); 
    println!();
    println!("---");
    println!(); 

    session.history.push(Interaction {
        query: query.clone(),
        response: Some(full_response),
    });

    let session_json = serde_json::to_string_pretty(&session)?;
    std::fs::write(&convo_path, session_json)?;

    Ok(())
}
