/*
 * vecdb-common: Shared Utilities for the vecdb Ecosystem
 *
 * PURPOSE:
 *   Provides common patterns and utilities used across vecq, vecdb-core,
 *   vecdb-cli, and vecdb-server. Designed to be minimal and dependency-free.
 *
 * PHILOSOPHY:
 *   See docs/planning/PHILOSOPHY.md - "Codified Correctness"
 *   This crate embodies the principle: correctness through structure, not discipline.
 *
 * MODULES:
 *   - output: TTY-aware output handling (OutputContext pattern)
 *   - input: Stdin-aware input handling (InputContext pattern)
 */

pub mod detection;
pub mod input;
pub mod lines;
pub mod logging;
pub mod output;
pub mod text;

// Re-export commonly used items for ergonomics
pub use detection::{FileType, FileTypeDetector, ParsingCapability};
pub use input::{InputContext, INPUT};
pub use lines::LineCounter;
pub use output::{OutputContext, OUTPUT};
pub use text::stitch_text;
