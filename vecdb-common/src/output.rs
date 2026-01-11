/*
 * PURPOSE:
 *   Centralized output context for TTY-aware formatting.
 *   Implements the "Codified Correctness" philosophy - see docs/planning/PHILOSOPHY.md
 *
 * ⚠️ RULE: OUTPUT
 *   All functions producing user-facing output should receive OutputContext
 *   or access the global OUTPUT singleton. Check `ctx.is_interactive` before
 *   emitting progress messages, colors, or ANSI codes.
 *
 * USAGE:
 *   use vecdb_common::{OutputContext, OUTPUT};
 *   
 *   // Option A: Use global singleton
 *   if OUTPUT.is_interactive {
 *       eprintln!("Processing...");
 *   }
 *   
 *   // Option B: Pass as parameter (preferred for public APIs)
 *   fn my_command(ctx: &OutputContext) {
 *       if ctx.is_interactive {
 *           // fancy output
 *       }
 *   }
 *
 * TESTING:
 *   See vecdb-common/TESTING.md for test strategies.
 */

use std::io::IsTerminal;
use std::sync::LazyLock;

/// Global output configuration, detected once at startup.
pub static OUTPUT: LazyLock<OutputContext> = LazyLock::new(OutputContext::detect);

/// Runtime output context for TTY-aware formatting.
/// 
/// # Example
/// ```
/// use vecdb_common::OutputContext;
/// 
/// let ctx = OutputContext::detect();
/// if ctx.is_interactive {
///     println!("Hello, human!");
/// }
/// ```
#[derive(Debug, Clone)]
pub struct OutputContext {
    /// True if stdout is connected to an interactive terminal.
    /// When false, suppress progress messages, colors, and interactive elements.
    pub is_interactive: bool,
    
    /// Explicit override for colors (None = auto-detect based on is_interactive)
    pub color_override: Option<bool>,
}

impl OutputContext {
    /// Detect the output context from the current environment.
    /// 
    /// Checks:
    /// 1. Is stderr connected to a TTY? (Used for interactivity/progress)
    /// 2. Is stdout connected to a TTY? (Used for color detection)
    /// 3. Is the NO_COLOR environment variable set?
    pub fn detect() -> Self {
        let stdout_tty = std::io::stdout().is_terminal();
        let stderr_tty = std::io::stderr().is_terminal();
        
        // Respect NO_COLOR environment variable (https://no-color.org/)
        let no_color = std::env::var("NO_COLOR")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        
        Self {
            // Interactive features (progress bars, status updates) should follow stderr.
            // If you pipe stdout to a file, you still want to see progress on your screen (stderr).
            is_interactive: stderr_tty,
            // Color detection is trickier. If we are printing to stdout, we follow stdout.
            // But usually, we want to know if colors are supported AT ALL in the current session.
            color_override: if no_color { Some(false) } else if !stdout_tty { Some(false) } else { None },
        }
    }
    
    /// Check if colors should be used for stdout.
    /// 
    /// Returns false if:
    /// - Output is not a TTY (piped or redirected)
    /// - NO_COLOR environment variable is set
    /// - Color was explicitly disabled via `color_override`
    pub fn use_color(&self) -> bool {
        self.color_override.unwrap_or(self.is_interactive)
    }
    
    /// Create a non-interactive context (for testing or forced quiet mode).
    /// 
    /// # Example
    /// ```
    /// use vecdb_common::OutputContext;
    /// 
    /// let ctx = OutputContext::quiet();
    /// assert!(!ctx.is_interactive);
    /// assert!(!ctx.use_color());
    /// ```
    pub fn quiet() -> Self {
        Self {
            is_interactive: false,
            color_override: Some(false),
        }
    }
    
    /// Create an interactive context (for testing or forced verbose mode).
    /// 
    /// # Example
    /// ```
    /// use vecdb_common::OutputContext;
    /// 
    /// let ctx = OutputContext::interactive();
    /// assert!(ctx.is_interactive);
    /// assert!(ctx.use_color());
    /// ```
    pub fn interactive() -> Self {
        Self {
            is_interactive: true,
            color_override: None,
        }
    }
    
    /// Create a context with explicit color control.
    pub fn with_color(use_color: bool) -> Self {
        Self {
            is_interactive: use_color,
            color_override: Some(use_color),
        }
    }
}

impl Default for OutputContext {
    fn default() -> Self {
        Self::detect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_quiet_context() {
        let ctx = OutputContext::quiet();
        assert!(!ctx.is_interactive);
        assert!(!ctx.use_color());
    }
    
    #[test]
    fn test_interactive_context() {
        let ctx = OutputContext::interactive();
        assert!(ctx.is_interactive);
        assert!(ctx.use_color());
    }
    
    #[test]
    fn test_with_color_true() {
        let ctx = OutputContext::with_color(true);
        assert!(ctx.use_color());
    }
    
    #[test]
    fn test_with_color_false() {
        let ctx = OutputContext::with_color(false);
        assert!(!ctx.use_color());
    }
    
    #[test]
    fn test_color_override_takes_precedence() {
        let mut ctx = OutputContext::interactive();
        ctx.color_override = Some(false);
        assert!(!ctx.use_color()); // Override wins
    }
    
    #[test]
    fn test_default_is_detect() {
        // Note: This test's behavior depends on the test runner's TTY state
        let ctx = OutputContext::default();
        // Can't assert is_interactive value, but we can check it doesn't panic
        let _ = ctx.use_color();
    }
}
