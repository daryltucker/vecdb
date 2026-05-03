pub mod arbitrated;
pub mod local;
pub mod ollama;

pub use arbitrated::ArbitratedEmbedder;
pub use ollama::OllamaEmbedder;

pub mod mock;
pub use mock::MockEmbedder;

#[cfg(feature = "local-embed")]
pub use local::LocalEmbedder;
