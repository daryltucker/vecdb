pub mod local;
pub mod ollama;

pub use ollama::OllamaEmbedder;

pub mod mock;
pub use mock::MockEmbedder;

#[cfg(feature = "local-embed")]
pub use local::LocalEmbedder;
