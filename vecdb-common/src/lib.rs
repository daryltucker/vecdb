/*
 * vecdb-common: Shared Utilities for the vecdb Ecosystem
 *
 * PURPOSE:
 *   Provides common patterns and utilities used across vecq, vecdb-core,
 *   vecdb-cli, and vecdb-server. Designed to be minimal and dependency-free.
 *
 * PHILOSOPHY — "Codified Correctness":
 *   Correctness through structure, not discipline. Where a rule can be made
 *   unrepresentable it is; where it cannot, it is checked. Nothing here relies
 *   on a caller remembering to do the right thing.
 *
 * MODULES:
 *   - output: TTY-aware output handling (OutputContext pattern)
 *   - input: Stdin-aware input handling (InputContext pattern)
 *   - version: build-time git revision, stamped by build.rs
 */

pub mod detection;
pub mod input;
pub mod lines;
pub mod logging;
pub mod output;
pub mod text;
pub mod version;

// Re-export commonly used items for ergonomics
pub use detection::{FileType, FileTypeDetector, ParsingCapability};
pub use input::{InputContext, INPUT};
pub use lines::LineCounter;
pub use output::{OutputContext, OUTPUT};
pub use text::stitch_text;
pub use version::{revision, short_version, GIT_HASH};
