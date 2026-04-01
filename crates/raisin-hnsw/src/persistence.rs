// SPDX-License-Identifier: BSL-1.1

//! Persistence for HNSW indexes using usearch native format + JSON metadata sidecar.
//!
//! Two files per index:
//! - `{key}.hnsw` — usearch native index (graph + vectors)
//! - `{key}.hnsw.meta` — JSON metadata sidecar (node mappings + config)

use crate::index::{HnswIndex, NodeMeta};
use crate::migration;
use crate::types::DistanceMetric;
use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Metadata sidecar format stored alongside the usearch native index.
#[derive(Serialize, Deserialize)]
pub(crate) struct IndexMetadata {
    pub node_to_key: HashMap<String, u64>,
    pub key_to_meta: HashMap<u64, NodeMeta>,
    pub dimensions: usize,
    pub distance_metric: DistanceMetric,
    pub next_key: u64,
}

/// Save an HNSW index to disk as dual files (.hnsw + .hnsw.meta).
pub(crate) fn save_to_file(index: &HnswIndex, path: &Path) -> Result<()> {
    // Create parent directory if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to create directory: {}", e))
        })?;
    }

    // Save usearch index natively
    let path_str = path.to_str().ok_or_else(|| {
        raisin_error::Error::storage("Index path contains invalid UTF-8".to_string())
    })?;
    index.usearch_index().save(path_str).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to save usearch index: {}", e))
    })?;

    // Save metadata sidecar
    let meta_path = meta_path_for(path);
    let metadata = IndexMetadata {
        node_to_key: index.node_to_key().clone(),
        key_to_meta: index.key_to_meta().clone(),
        dimensions: index.dimensions(),
        distance_metric: index.distance_metric(),
        next_key: index.next_key(),
    };
    let json = serde_json::to_vec(&metadata).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to serialize index metadata: {}", e))
    })?;
    std::fs::write(&meta_path, json).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to write metadata sidecar: {}", e))
    })?;

    tracing::debug!(
        path = %path.display(),
        count = index.len(),
        "Saved HNSW index (usearch + metadata)"
    );

    Ok(())
}

/// Load an HNSW index from disk, auto-detecting old vs new format.
///
/// If a `.hnsw.meta` sidecar exists, loads the new usearch format.
/// Otherwise, falls back to migrating from the old bincode format.
pub(crate) fn load_from_file(path: &Path) -> Result<HnswIndex> {
    let meta_path = meta_path_for(path);

    if meta_path.exists() {
        load_new_format(path, &meta_path)
    } else {
        migration::migrate_from_old_format(path)
    }
}

/// Load index from new dual-file format.
fn load_new_format(path: &Path, meta_path: &Path) -> Result<HnswIndex> {
    // Load metadata sidecar
    let meta_bytes = std::fs::read(meta_path).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to read metadata sidecar: {}", e))
    })?;
    let metadata: IndexMetadata = serde_json::from_slice(&meta_bytes).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to deserialize metadata: {}", e))
    })?;

    // Reconstruct usearch index with same options
    let index = HnswIndex::from_persisted(
        path,
        metadata.dimensions,
        metadata.distance_metric,
        metadata.node_to_key,
        metadata.key_to_meta,
        metadata.next_key,
    )?;

    tracing::debug!(
        path = %path.display(),
        count = index.len(),
        dims = metadata.dimensions,
        "Loaded HNSW index (usearch + metadata)"
    );

    Ok(index)
}

/// View (mmap) an HNSW index from disk, auto-detecting old vs new format.
///
/// The usearch graph is memory-mapped and read-only. Metadata sidecar is
/// still loaded into RAM. Old format falls back to full migration.
pub(crate) fn view_from_file(path: &Path) -> Result<HnswIndex> {
    let meta_path = meta_path_for(path);

    if meta_path.exists() {
        view_new_format(path, &meta_path)
    } else {
        // Old format cannot be mmap'd — fall back to migration (fully loads)
        migration::migrate_from_old_format(path)
    }
}

/// View index from new dual-file format using memory mapping.
fn view_new_format(path: &Path, meta_path: &Path) -> Result<HnswIndex> {
    let meta_bytes = std::fs::read(meta_path).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to read metadata sidecar: {}", e))
    })?;
    let metadata: IndexMetadata = serde_json::from_slice(&meta_bytes).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to deserialize metadata: {}", e))
    })?;

    let index = HnswIndex::from_persisted_view(
        path,
        metadata.dimensions,
        metadata.distance_metric,
        metadata.node_to_key,
        metadata.key_to_meta,
        metadata.next_key,
    )?;

    tracing::debug!(
        path = %path.display(),
        count = index.len(),
        dims = metadata.dimensions,
        "Viewed (mmap) HNSW index"
    );

    Ok(index)
}

/// Compute the metadata sidecar path for a given index path.
pub(crate) fn meta_path_for(path: &Path) -> std::path::PathBuf {
    let mut meta = path.as_os_str().to_os_string();
    meta.push(".meta");
    std::path::PathBuf::from(meta)
}
