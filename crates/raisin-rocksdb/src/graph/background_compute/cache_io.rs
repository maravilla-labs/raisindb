//! Cache I/O operations for the graph computation background task.
//!
//! Handles reading/writing cache metadata and entries in the GRAPH_CACHE
//! column family, staleness checking, and recomputation orchestration.

use super::GraphComputeTask;
use crate::graph::{
    cache_layer::GraphCacheLayer,
    config::GraphAlgorithmConfig,
    types::{CacheStatus, GraphCacheMeta, GraphCacheValue, TargetMode},
    AlgorithmExecutor,
};
use crate::keys::{graph_cache_branch_prefix, graph_cache_key, graph_cache_meta_key};
use crate::{cf, cf_handle, RocksDBStorage};
use raisin_error::{Error, Result};
use rocksdb::WriteBatch;
use std::collections::HashMap;

impl GraphComputeTask {
    /// Check if a config needs recomputation for a branch
    pub(super) async fn needs_recomputation(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config: &GraphAlgorithmConfig,
    ) -> Result<bool> {
        // Read metadata from GRAPH_CACHE column family
        let meta = Self::read_cache_meta(storage, tenant_id, repo_id, branch_id, &config.id)?;

        match meta {
            None => {
                // No cache exists, needs computation
                Ok(true)
            }
            Some(meta) => {
                // Check if marked as stale
                if meta.status == CacheStatus::Stale {
                    return Ok(true);
                }

                // Check TTL expiration
                if config.refresh.ttl_seconds > 0 {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;

                    if meta.next_scheduled_at > 0 && now >= meta.next_scheduled_at {
                        return Ok(true);
                    }
                }

                // Check if branch HEAD has changed (for branch-tracking configs)
                if config.refresh.on_branch_change && config.is_branch_tracking() {
                    let current_head =
                        Self::get_branch_head(storage, tenant_id, repo_id, branch_id).await?;
                    if current_head != meta.revision_id {
                        return Ok(true);
                    }
                }

                Ok(false)
            }
        }
    }

    /// Recompute graph algorithm for a specific branch
    ///
    /// This method is public to allow manual recomputation via API.
    pub async fn recompute_for_branch(
        storage: &RocksDBStorage,
        cache_layer: &GraphCacheLayer,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config: &GraphAlgorithmConfig,
        max_nodes: usize,
    ) -> Result<usize> {
        let start = std::time::Instant::now();

        // Get current branch HEAD revision
        let revision = Self::get_branch_head(storage, tenant_id, repo_id, branch_id).await?;

        // Build graph projection from scoped nodes
        let projection = Self::build_projection(
            storage,
            tenant_id,
            repo_id,
            branch_id,
            &revision,
            &config.scope,
            max_nodes,
        )
        .await?;

        let node_count = projection.node_count();

        if node_count == 0 {
            tracing::debug!(
                config_id = %config.id,
                "No nodes in scope, skipping computation"
            );
            return Ok(0);
        }

        // Execute the algorithm
        let result = AlgorithmExecutor::execute(config, &projection)?;

        // Build cache entries
        let ttl_seconds = config.refresh.ttl_seconds;
        let cache_entries = AlgorithmExecutor::build_cache_entries(
            result,
            &revision,
            &config.id, // Use config ID as config revision
            ttl_seconds,
        );

        // Write results to GRAPH_CACHE column family
        Self::write_cache_entries(
            storage,
            tenant_id,
            repo_id,
            branch_id,
            &config.id,
            &cache_entries,
        )?;

        // Update cache metadata
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let next_scheduled = if ttl_seconds > 0 {
            now + (ttl_seconds * 1000)
        } else {
            0
        };

        let meta = GraphCacheMeta {
            target_mode: TargetMode::Branch,
            branch_id: Some(branch_id.to_string()),
            revision_id: revision.clone(),
            last_computed_at: now,
            next_scheduled_at: next_scheduled,
            node_count: node_count as u64,
            status: CacheStatus::Ready,
            error: None,
        };

        Self::write_cache_meta(storage, tenant_id, repo_id, branch_id, &config.id, &meta)?;

        // Invalidate in-memory cache to force reload from RocksDB
        cache_layer.invalidate(&config.id);

        let duration_ms = start.elapsed().as_millis();
        tracing::debug!(
            config_id = %config.id,
            node_count = node_count,
            duration_ms = duration_ms,
            "Graph algorithm computation completed"
        );

        Ok(node_count)
    }

    /// Get the current HEAD revision for a branch
    pub(super) async fn get_branch_head(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
    ) -> Result<String> {
        use raisin_storage::BranchRepository;

        let head = storage
            .branches_impl()
            .get_head(tenant_id, repo_id, branch_id)
            .await?;

        Ok(head.to_string())
    }

    /// Read cache metadata from GRAPH_CACHE CF
    pub(super) fn read_cache_meta(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config_id: &str,
    ) -> Result<Option<GraphCacheMeta>> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_CACHE)?;

        let key = graph_cache_meta_key(tenant_id, repo_id, branch_id, config_id);

        match db.get_cf(cf, &key) {
            Ok(Some(bytes)) => {
                let meta: GraphCacheMeta = rmp_serde::from_slice(&bytes)
                    .map_err(|e| Error::storage(format!("Failed to deserialize meta: {}", e)))?;
                Ok(Some(meta))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(Error::storage(format!("Failed to read cache meta: {}", e))),
        }
    }

    /// Write cache metadata to GRAPH_CACHE CF
    fn write_cache_meta(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config_id: &str,
        meta: &GraphCacheMeta,
    ) -> Result<()> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_CACHE)?;

        let key = graph_cache_meta_key(tenant_id, repo_id, branch_id, config_id);
        let value = rmp_serde::to_vec(meta)
            .map_err(|e| Error::storage(format!("Failed to serialize meta: {}", e)))?;

        db.put_cf(cf, key, value)
            .map_err(|e| Error::storage(format!("Failed to write cache meta: {}", e)))?;

        Ok(())
    }

    /// Write cache entries to GRAPH_CACHE CF
    fn write_cache_entries(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config_id: &str,
        entries: &HashMap<String, GraphCacheValue>,
    ) -> Result<()> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_CACHE)?;

        let mut batch = WriteBatch::default();

        for (node_id, value) in entries {
            let key = graph_cache_key(tenant_id, repo_id, branch_id, config_id, node_id);
            let value_bytes = rmp_serde::to_vec(value)
                .map_err(|e| Error::storage(format!("Failed to serialize value: {}", e)))?;
            batch.put_cf(cf, key, value_bytes);
        }

        db.write(batch)
            .map_err(|e| Error::storage(format!("Failed to write cache entries: {}", e)))?;

        Ok(())
    }

    /// Mark cache as stale for a config/branch
    /// Called when relations change within scope
    pub fn mark_stale(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config_id: &str,
    ) -> Result<()> {
        let meta = Self::read_cache_meta(storage, tenant_id, repo_id, branch_id, config_id)?;

        if let Some(mut meta) = meta {
            meta.status = CacheStatus::Stale;
            Self::write_cache_meta(storage, tenant_id, repo_id, branch_id, config_id, &meta)?;

            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch_id = %branch_id,
                config_id = %config_id,
                "Marked graph cache as stale"
            );
        }

        Ok(())
    }

    /// Read cache metadata from GRAPH_CACHE CF (public API)
    ///
    /// Returns the current cache metadata for a specific config/branch combination.
    pub fn get_cache_meta(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
        config_id: &str,
    ) -> Result<Option<GraphCacheMeta>> {
        Self::read_cache_meta(storage, tenant_id, repo_id, branch_id, config_id)
    }

    /// Mark all caches stale for a specific branch (called on branch HEAD change)
    pub fn mark_branch_stale(
        storage: &RocksDBStorage,
        tenant_id: &str,
        repo_id: &str,
        branch_id: &str,
    ) -> Result<()> {
        let db = storage.db();
        let cf = cf_handle(db, cf::GRAPH_CACHE)?;

        // Prefix for all entries in this branch
        let prefix = graph_cache_branch_prefix(tenant_id, repo_id, branch_id);

        let iter = db.prefix_iterator_cf(cf, &prefix);
        let mut batch = WriteBatch::default();
        let mut count = 0;

        for result in iter {
            let (key, value) =
                result.map_err(|e| Error::storage(format!("Failed to iterate cache: {}", e)))?;

            // Only process _meta keys
            if !key.ends_with(b"_meta") {
                continue;
            }

            // Deserialize, mark stale, serialize back
            let mut meta: GraphCacheMeta = rmp_serde::from_slice(&value)
                .map_err(|e| Error::storage(format!("Failed to deserialize meta: {}", e)))?;

            if meta.status != CacheStatus::Stale {
                meta.status = CacheStatus::Stale;
                let value_bytes = rmp_serde::to_vec(&meta)
                    .map_err(|e| Error::storage(format!("Failed to serialize meta: {}", e)))?;
                batch.put_cf(cf, key, value_bytes);
                count += 1;
            }
        }

        if count > 0 {
            db.write(batch)
                .map_err(|e| Error::storage(format!("Failed to mark stale: {}", e)))?;

            tracing::debug!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch_id = %branch_id,
                count = count,
                "Marked graph caches as stale"
            );
        }

        Ok(())
    }
}
