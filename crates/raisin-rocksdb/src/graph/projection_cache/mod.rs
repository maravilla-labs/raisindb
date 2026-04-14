// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Persistent graph projection storage
//!
//! Stores serialized graph projections in the GRAPH_PROJECTION column family.
//! Projections are loaded on demand when algorithms need them. RocksDB's
//! block cache handles hot-data caching -- no separate in-memory cache needed.

use crate::graph::types::PersistedProjection;
use crate::keys::{graph_projection_branch_prefix, graph_projection_key};
use crate::{cf, cf_handle, RocksDBStorage};
use raisin_graph_algorithms::GraphProjection;
use rocksdb::WriteBatch;

#[cfg(test)]
mod tests;

/// Key for stored projections: (tenant, repo, branch, config_id)
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ProjectionKey {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub config_id: String,
}

/// Persistent graph projection store backed by the GRAPH_PROJECTION column family.
///
/// No in-memory cache -- RocksDB's block cache handles repeated reads.
/// The event handler marks projections stale on relation changes,
/// and the background compute rebuilds them lazily.
pub struct GraphProjectionStore;

impl GraphProjectionStore {
    /// Load a projection from RocksDB. Returns None if not found or stale.
    pub fn load(
        key: &ProjectionKey,
        storage: &RocksDBStorage,
    ) -> Result<Option<GraphProjection>, String> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_PROJECTION)
            .map_err(|e| format!("Failed to get GRAPH_PROJECTION CF: {}", e))?;

        let raw_key =
            graph_projection_key(&key.tenant_id, &key.repo_id, &key.branch, &key.config_id);

        match db.get_cf(cf, &raw_key) {
            Ok(Some(bytes)) => {
                let persisted: PersistedProjection = rmp_serde::from_slice(&bytes)
                    .map_err(|e| format!("Failed to deserialize projection: {}", e))?;

                if persisted.is_stale() {
                    return Ok(None);
                }

                Ok(Some(persisted.to_projection()))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("Failed to read projection: {}", e)),
        }
    }

    /// Load persisted metadata (without building the full GraphProjection).
    /// Useful for checking revision without the cost of CSR construction.
    pub fn load_meta(
        key: &ProjectionKey,
        storage: &RocksDBStorage,
    ) -> Result<Option<PersistedProjection>, String> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_PROJECTION)
            .map_err(|e| format!("Failed to get GRAPH_PROJECTION CF: {}", e))?;

        let raw_key =
            graph_projection_key(&key.tenant_id, &key.repo_id, &key.branch, &key.config_id);

        match db.get_cf(cf, &raw_key) {
            Ok(Some(bytes)) => {
                let persisted: PersistedProjection = rmp_serde::from_slice(&bytes)
                    .map_err(|e| format!("Failed to deserialize projection: {}", e))?;
                Ok(Some(persisted))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("Failed to read projection: {}", e)),
        }
    }

    /// Store a projection in RocksDB.
    pub fn store(
        key: &ProjectionKey,
        projection: &GraphProjection,
        revision: String,
        storage: &RocksDBStorage,
    ) -> Result<(), String> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_PROJECTION)
            .map_err(|e| format!("Failed to get GRAPH_PROJECTION CF: {}", e))?;

        let persisted = PersistedProjection::from_projection(projection, revision);
        let bytes = rmp_serde::to_vec_named(&persisted)
            .map_err(|e| format!("Failed to serialize projection: {}", e))?;

        let raw_key =
            graph_projection_key(&key.tenant_id, &key.repo_id, &key.branch, &key.config_id);

        db.put_cf(cf, raw_key, bytes)
            .map_err(|e| format!("Failed to write projection: {}", e))?;

        Ok(())
    }

    /// Mark a single projection as stale.
    pub fn mark_stale(key: &ProjectionKey, storage: &RocksDBStorage) -> Result<(), String> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_PROJECTION)
            .map_err(|e| format!("Failed to get GRAPH_PROJECTION CF: {}", e))?;

        let raw_key =
            graph_projection_key(&key.tenant_id, &key.repo_id, &key.branch, &key.config_id);

        match db.get_cf(cf, &raw_key) {
            Ok(Some(bytes)) => {
                let mut persisted: PersistedProjection = rmp_serde::from_slice(&bytes)
                    .map_err(|e| format!("Failed to deserialize: {}", e))?;

                if !persisted.is_stale() {
                    persisted.mark_stale();
                    let new_bytes = rmp_serde::to_vec_named(&persisted)
                        .map_err(|e| format!("Failed to serialize: {}", e))?;
                    db.put_cf(cf, raw_key, new_bytes)
                        .map_err(|e| format!("Failed to write: {}", e))?;
                }
            }
            Ok(None) => {} // Nothing to mark
            Err(e) => return Err(format!("Failed to read: {}", e)),
        }

        Ok(())
    }

    /// Mark all projections for a branch as stale.
    pub fn mark_branch_stale(
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        storage: &RocksDBStorage,
    ) -> Result<(), String> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_PROJECTION)
            .map_err(|e| format!("Failed to get GRAPH_PROJECTION CF: {}", e))?;

        let prefix = graph_projection_branch_prefix(tenant_id, repo_id, branch);
        let iter = db.prefix_iterator_cf(cf, &prefix);
        let mut batch = WriteBatch::default();
        let mut count = 0;

        for result in iter {
            let (key, value) =
                result.map_err(|e| format!("Failed to iterate projections: {}", e))?;

            if !key.starts_with(&prefix) {
                break;
            }

            let mut persisted: PersistedProjection = match rmp_serde::from_slice(&value) {
                Ok(p) => p,
                Err(_) => continue,
            };

            if !persisted.is_stale() {
                persisted.mark_stale();
                if let Ok(new_bytes) = rmp_serde::to_vec_named(&persisted) {
                    batch.put_cf(cf, &key, new_bytes);
                    count += 1;
                }
            }
        }

        if count > 0 {
            db.write(batch)
                .map_err(|e| format!("Failed to write stale projections: {}", e))?;
        }

        Ok(())
    }

    /// Delete a projection from storage.
    pub fn delete(key: &ProjectionKey, storage: &RocksDBStorage) -> Result<(), String> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_PROJECTION)
            .map_err(|e| format!("Failed to get GRAPH_PROJECTION CF: {}", e))?;

        let raw_key =
            graph_projection_key(&key.tenant_id, &key.repo_id, &key.branch, &key.config_id);

        db.delete_cf(cf, raw_key)
            .map_err(|e| format!("Failed to delete projection: {}", e))?;

        Ok(())
    }
}
