//! Scanning, extraction and embedding of the watched folders.

use anyhow::{Result, bail};
use rummage_core::Service;

/// Indexes new and changed files in every enabled folder once, then returns.
pub fn run_once(_svc: &Service) -> Result<()> {
    bail!(
        "indexing is not part of this build yet; folders and settings are saved and will be indexed once it is"
    )
}

/// Indexes, then keeps watching the folders for changes until stopped.
pub async fn watch(_svc: std::sync::Arc<Service>) -> Result<()> {
    bail!(
        "indexing is not part of this build yet; folders and settings are saved and will be indexed once it is"
    )
}
