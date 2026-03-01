pub mod strategy;
mod types;

pub use strategy::state::{process_state, FileSystemSnapshotLoader};
pub use strategy::stream::process_stream;
pub use types::*;
