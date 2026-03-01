/*
 * PURPOSE:
 *   Implements the `man` command to display embedded documentation.
 *   Supports dual-mode output: Rich Text for Humans, Raw Text for Agents.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - "man" and "man --agent" support (User Prompt)
 *   - "Grep-like" feel (Native, fast)
 *
 * IMPLEMENTATION RULES:
 *   1. Use `include_str!`
 *      Rationale: Zero-dependency runtime; docs are part of the binary.
 *
 *   2. Use `termimad` for human rendering
 *      Rationale: Provides nice formatting (bold, headers) in the terminal.
 *
 *   3. Stdout for content
 *      Rationale: Allows piping `vecdb man | grep ...`
 */

use clap::Args;
use termimad::MadSkin;

#[derive(Args, Debug)]
pub struct ManArgs {
    /// The specific command to view manual for (e.g., "ingest")
    #[arg(index = 1)]
    pub command: Option<String>,

    /// Output raw, machine-readable documentation for Agents
    #[arg(long)]
    pub agent: bool,
}

const MAN_HUMAN: &str = include_str!("../docs/man_human.md");
const MAN_AGENT: &str = include_str!("../docs/man_agent.md");

pub fn run(args: ManArgs) -> anyhow::Result<()> {
    let content = if args.agent { MAN_AGENT } else { MAN_HUMAN };

    // If a specific command is requested, try to extract its section
    let output_text = if let Some(cmd) = args.command {
        extract_section(content, &cmd).unwrap_or_else(|| {
            format!(
                "No manual entry found for '{}'. Showing full manual.\n\n{}",
                cmd, content
            )
        })
    } else {
        content.to_string()
    };

    if args.agent {
        // Raw output for agents/piping
        println!("{}", output_text);
    } else {
        // Rich output for humans
        let skin = make_sexy_skin();
        skin.print_text(&output_text);
    }
    Ok(())
}

fn make_sexy_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.headers[0].set_fg(termimad::crossterm::style::Color::Cyan); // H1
    skin.headers[1].set_fg(termimad::crossterm::style::Color::Magenta); // H2
    skin.headers[2].set_fg(termimad::crossterm::style::Color::Yellow); // H3
    skin.bold.set_fg(termimad::crossterm::style::Color::Green);
    skin.italic
        .set_fg(termimad::crossterm::style::Color::DarkGrey);
    skin
}

/// Simple parser to find a header containing the command name and extract text until next same-level header
fn extract_section(full_text: &str, command: &str) -> Option<String> {
    let query = command.to_lowercase();
    let lines: Vec<&str> = full_text.lines().collect();
    let mut matching = false;
    let mut result = Vec::new();
    let mut level = 0;

    for line in lines {
        if line.starts_with('#') {
            let current_level = line.chars().take_while(|c| *c == '#').count();
            let lower_line = line.to_lowercase();

            // Check if this header matches our command
            // We look for "ingest" in "### Essential", "### ingest", "* **ingest**" (list item?)
            // Implementation detail: The docs structure needs to match.
            // For now, let's look for "### ... command ..." or just sloppy match on header

            if matching {
                // If we hit a header of same or higher importance, stop
                if current_level <= level {
                    break;
                }
            } else {
                // Check match
                if lower_line.contains(&query) {
                    matching = true;
                    level = current_level;
                }
            }
        }

        // Also check for bold list items which `man_human.md` uses: "* **ingest**"
        if !matching && line.trim().starts_with('*') && line.to_lowercase().contains(&query) {
            matching = true;
            level = 99; // Treat list items as leaf nodes
        } else if matching && level == 99 && line.trim().is_empty() {
            // For list items, maybe stop at double newline? Or next list item?
            // Let's rely on standard markdown headers for now for "Sections"
        }

        if matching {
            result.push(line);
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(result.join("\n"))
    }
}
