// SPDX-License-Identifier: BSL-1.1

//! Backward-compatible migration from old bincode index format.
//!
//! The old format used `instant-distance` with bincode serialization.
//! This module deserializes old indexes and re-inserts vectors into the
//! new usearch-backed index. After migration, the index is automatically
//! saved in the new dual-file format (.hnsw + .hnsw.meta).
//!
//! This module can be removed once all deployed indexes have been migrated.

use crate::index::HnswIndex;
use crate::types::{DistanceMetric, VectorPoint};
use raisin_error::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Old serialized index format (bincode, instant-distance era).
///
/// Kept only for deserialization during migration.
#[derive(Deserialize)]
pub(crate) struct OldSerializedIndex {
    pub points: Vec<VectorPoint>,
    pub node_to_point: HashMap<String, usize>,
    pub dimensions: usize,
    #[allow(dead_code)]
    pub count: usize,
    #[serde(default)]
    pub distance_metric: DistanceMetric,
}

/// Migrate an old bincode-format index to the new usearch format.
///
/// Reads the old file, creates a new usearch index, re-inserts all active
/// vectors, and saves in the new dual-file format. The old file is replaced.
pub(crate) fn migrate_from_old_format<P: AsRef<Path>>(path: P) -> Result<HnswIndex> {
    let path = path.as_ref();
    tracing::info!(
        path = %path.display(),
        "Migrating HNSW index from old bincode format to usearch native format"
    );

    let bytes = std::fs::read(path).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to read old index file: {}", e))
    })?;

    let old: OldSerializedIndex = bincode::deserialize(&bytes).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to deserialize old index format: {}", e))
    })?;

    let mut new_index = HnswIndex::with_metric(old.dimensions, old.distance_metric);

    // Re-insert only active points (those still referenced in node_to_point)
    for point in &old.points {
        if old.node_to_point.contains_key(&point.node_id) {
            new_index.add(
                point.node_id.clone(),
                point.workspace_id.clone(),
                point.revision,
                point.vector.clone(),
            )?;
        }
    }

    // Save in new format (auto-upgrade)
    new_index.save_to_file(path)?;

    tracing::info!(
        count = new_index.len(),
        dims = old.dimensions,
        metric = %old.distance_metric,
        "Migration complete"
    );

    Ok(new_index)
}
