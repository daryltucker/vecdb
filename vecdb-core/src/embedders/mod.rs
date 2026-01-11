pub mod ollama;
pub mod local;

pub use ollama::OllamaEmbedder;

#[cfg(feature = "local-embed")]
pub use local::LocalEmbedder;
