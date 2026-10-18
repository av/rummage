//! The parts of rummage that don't depend on HTTP or the model runtime: config, the SQLite
//! store, API types, search, and the [`embed::Embedder`] trait the engine implements.

pub mod config;
pub mod embed;
pub mod service;
pub mod store;
pub mod types;

pub use config::{Config, ConfigFile};
pub use embed::{EmbedInput, Embedder, FakeEmbedder, ImageData, Modalities};
pub use service::Service;
pub use store::Store;
