/*
 * PURPOSE:
 *   Centralized input context for stdin-aware processing.
 *   Implements the "Codified Correctness" philosophy - see docs/planning/PHILOSOPHY.md
 *
 * ⚠️ RULE: INPUT
 *   CLI tools should automatically detect stdin when:
 *   1. No input file/path is provided
 *   2. Stdin is not a TTY (data is being piped in)
 *
 * USAGE:
 *   use vecdb_common::{InputContext, INPUT};
 *
 *   if INPUT.has_piped_data {
 *       // Read from stdin
 *   } else if args.input.is_none() {
 *       // Show help - no input provided
 *   }
 *
 * TESTING:
 *   See vecdb-common/TESTING.md for test strategies.
 */

use std::io::IsTerminal;
use std::sync::LazyLock;

/// Global input configuration, detected once at startup.
pub static INPUT: LazyLock<InputContext> = LazyLock::new(InputContext::detect);

/// Runtime input context for stdin-aware processing.
///
/// # Example
/// ```
/// use vecdb_common::InputContext;
///
/// let ctx = InputContext::detect();
/// if ctx.has_piped_data {
///     // Read from stdin
/// }
/// ```
#[derive(Debug, Clone)]
pub struct InputContext {
    /// True if stdin has piped data (not connected to a terminal).
    /// When true, the program should read from stdin.
    pub has_piped_data: bool,

    /// True if stdin is connected to an interactive terminal.
    pub stdin_is_tty: bool,
}

impl InputContext {
    /// Detect the input context from the current environment.
    ///
    /// Checks if stdin is connected to a TTY:
    /// - TTY: User is typing interactively (or no input)
    /// - Not TTY: Data is being piped from another command
    pub fn detect() -> Self {
        let stdin_is_tty = std::io::stdin().is_terminal();

        Self {
            has_piped_data: !stdin_is_tty,
            stdin_is_tty,
        }
    }

    /// Create a context indicating piped input (for testing).
    pub fn piped() -> Self {
        Self {
            has_piped_data: true,
            stdin_is_tty: false,
        }
    }

    /// Create a context indicating interactive input (for testing).
    pub fn interactive() -> Self {
        Self {
            has_piped_data: false,
            stdin_is_tty: true,
        }
    }
}

impl Default for InputContext {
    fn default() -> Self {
        Self::detect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_piped_context() {
        let ctx = InputContext::piped();
        assert!(ctx.has_piped_data);
        assert!(!ctx.stdin_is_tty);
    }

    #[test]
    fn test_interactive_context() {
        let ctx = InputContext::interactive();
        assert!(!ctx.has_piped_data);
        assert!(ctx.stdin_is_tty);
    }

    #[test]
    fn test_default_doesnt_panic() {
        let ctx = InputContext::default();
        let _ = ctx.has_piped_data;
    }
}
