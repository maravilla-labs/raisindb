// SPDX-License-Identifier: BSL-1.1

//! Persistence for HNSW indexes using usearch native format + JSON metadata sidecar.
//!
//! Two files per index:
//! - `{key}.hnsw` — usearch native index (graph + vectors)
//! - `{key}.hnsw.meta` — JSON metadata sidecar (node mappings + config)

use crate::index::{HnswIndex, NodeMeta};
use crate::migration;
use crate::types::{DistanceMetric, QuantizationType};
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
    /// Vector quantization type. Defaults to F32 for backward compatibility
    /// with sidecar files written before this field existed.
    #[serde(default)]
    pub quantization: QuantizationType,
}

/// Save an HNSW index to disk as dual files (.hnsw + .hnsw.meta).
///
/// Both files are written to temporaries next to their targets and RENAMED
/// into place, graph first and sidecar last. Writing in place truncated the
/// live files first, so any failure mid-write — a full disk, a crash, a
/// deleted directory — left zero-byte files behind, and every later load of
/// that index failed until someone removed them by hand. With the rename a
/// failed save leaves the previous files untouched.
pub(crate) fn save_to_file(index: &HnswIndex, path: &Path) -> Result<()> {
    // Create parent directory if needed
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to create directory: {}", e))
        })?;
    }

    let graph_tmp = tmp_path_for(path);
    let meta_path = meta_path_for(path);
    let meta_tmp = tmp_path_for(&meta_path);

    let written = (|| -> Result<()> {
        // Save usearch index natively
        let tmp_str = graph_tmp.to_str().ok_or_else(|| {
            raisin_error::Error::storage("Index path contains invalid UTF-8".to_string())
        })?;
        index.usearch_index().save(tmp_str).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to save usearch index: {}", e))
        })?;

        // Save metadata sidecar
        let metadata = IndexMetadata {
            node_to_key: index.node_to_key().clone(),
            key_to_meta: index.key_to_meta().clone(),
            dimensions: index.dimensions(),
            distance_metric: index.distance_metric(),
            next_key: index.next_key(),
            quantization: index.quantization(),
        };
        let json = serde_json::to_vec(&metadata).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize index metadata: {}", e))
        })?;
        std::fs::write(&meta_tmp, json).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to write metadata sidecar: {}", e))
        })?;

        std::fs::rename(&graph_tmp, path).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to move HNSW index into place: {}", e))
        })?;
        std::fs::rename(&meta_tmp, &meta_path).map_err(|e| {
            raisin_error::Error::storage(format!(
                "Failed to move metadata sidecar into place: {}",
                e
            ))
        })?;
        Ok(())
    })();

    if written.is_err() {
        let _ = std::fs::remove_file(&graph_tmp);
        let _ = std::fs::remove_file(&meta_tmp);
    }
    written?;

    tracing::debug!(
        path = %path.display(),
        count = index.len(),
        "Saved HNSW index (usearch + metadata)"
    );

    Ok(())
}

/// `<path>.tmp`: where a save writes before it renames into place.
fn tmp_path_for(path: &Path) -> std::path::PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    std::path::PathBuf::from(tmp)
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
        metadata.quantization,
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
        metadata.quantization,
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
