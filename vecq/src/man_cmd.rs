use termimad::MadSkin;
use vecdb_common::OUTPUT;

const MAN_HUMAN: &str = include_str!("docs/man_human.md");
const MAN_AGENT: &str = include_str!("docs/man_agent.md");

/// Display the manual page.
///
/// Output behavior (following the OUTPUT rule):
/// - `--agent` flag: Always raw markdown (for machine parsing)
/// - Piped/redirected: Raw markdown (no ANSI codes)
/// - Interactive terminal: Rich formatted output with colors
pub fn run(agent: bool, command: Option<String>) -> anyhow::Result<()> {
    let content = if agent { MAN_AGENT } else { MAN_HUMAN };

    // If a specific command is requested, try to extract its section
    let output_text = if let Some(cmd) = command {
        extract_section(content, &cmd).unwrap_or_else(|| {
            format!(
                "No manual entry found for '{}'. Showing full manual.\n\n{}",
                cmd, content
            )
        })
    } else {
        content.to_string()
    };

    // ⚠️ RULE: OUTPUT
    // Use rich output ONLY when:
    // 1. Not in agent mode (--agent flag)
    // 2. stdout is an interactive terminal (not piped/redirected)
    if agent || !OUTPUT.is_interactive {
        // Raw output for agents/piping/redirection
        println!("{}", output_text);
    } else {
        // Rich output for humans at a terminal
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
            // For list items, maybe stop at double newline?
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
