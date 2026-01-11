/*
 * PURPOSE:
 *   Module definition for vector database backends.
 *   Exposes available storage implementations.
 *
 * REQUIREMENTS:
 *   User-specified:
 *   - Pluggable architecture
 *
 * IMPLEMENTATION RULES:
 *   1. Feature-gating: Only expose backends enabled in Cargo.toml
 *      Rationale: Reduces compile times and dependencies for unused backends.
 */

#[cfg(feature = "qdrant")]
pub mod qdrant;
