//! Picks the embedding engine.

use std::sync::Arc;

use anyhow::Result;
use rummage_core::{Config, Embedder, FakeEmbedder};

/// The embedder for this config. The llama.cpp engine plugs in here; until then every build
/// uses the deterministic fake, so search results are not meaningful yet.
pub fn embedder(cfg: &Config) -> Result<Arc<dyn Embedder>> {
    Ok(Arc::new(FakeEmbedder::new(cfg.model.dim as usize)))
}

/// Backends compiled into this binary.
pub fn compiled_backends() -> Vec<&'static str> {
    [
        (cfg!(feature = "cpu"), "cpu"),
        (cfg!(feature = "vulkan"), "vulkan"),
        (cfg!(feature = "rocm"), "rocm"),
        (cfg!(feature = "cuda"), "cuda"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect()
}
