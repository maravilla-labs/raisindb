// SPDX-License-Identifier: BSL-1.1

//! Index mutation operations for the HNSW indexing engine.
//!
//! Provides methods for adding/removing embeddings, purging indexes,
//! creating indexes with a specific shape, and copying indexes for branch
//! operations.
//!
//! Every method here names a PARTITION. There is no "the index for this
//! branch" any more, because there never really was one: the `cf::EMBEDDINGS`
//! key has always carried an embedder hash and a kind, and collapsing them into
//! a single index is what made enabling a second embedder take the first one
//! down.

use crate::dims::IndexSpec;
use crate::index::HnswIndex;
use crate::partition::PartitionId;
use raisin_error::Result;
use raisin_hlc::HLC;
use std::sync::{Arc, RwLock};

use super::{HnswIndexingEngine, IndexKey};

impl HnswIndexingEngine {
    /// Add an embedding to one partition's index.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `partition` - Which embedding space this vector belongs to
    /// * `workspace_id` - Workspace identifier (stored as metadata, not in key)
    /// * `node_id` - Node identifier
    /// * `revision` - Revision (full HLC with timestamp and counter)
    /// * `embedding` - Embedding vector
    #[allow(clippy::too_many_arguments)]
    pub fn add_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        workspace_id: &str,
        node_id: &str,
        revision: HLC,
        embedding: Vec<f32>,
    ) -> Result<()> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;

        // Add to index (workspace_id is now stored as metadata)
        {
            let mut index = index_arc.write().unwrap();
            index.add(
                node_id.to_string(),
                workspace_id.to_string(),
                revision,
                embedding,
            )?;
        }

        let key = IndexKey::new(tenant_id, repo_id, branch, partition);
        self.mark_mutated(&key, &index_arc);

        self.metrics.record_embedding_added();

        Ok(())
    }

    /// Is this id currently in the partition's index?
    ///
    /// The embedding job asks before deciding a node needs no work. A stored
    /// row and an indexed vector are two different pieces of state that can
    /// disagree: HNSW snapshots lag (see `lifecycle.rs`), so a restart shortly
    /// after a write can lose the vector while the RocksDB row survives. If
    /// "already stored" alone were the skip condition, that node would be
    /// unsearchable until someone ran a full REBUILD; asking both means the very
    /// next job repairs it.
    pub fn contains_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        node_id: &str,
    ) -> Result<bool> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;
        let index = index_arc.read().unwrap();
        Ok(index.contains(node_id))
    }

    /// Remove an embedding from one partition's index.
    ///
    /// Note: workspace_id is no longer needed as parameter since all workspaces
    /// are in the same index. The node_id alone is sufficient for removal.
    pub fn remove_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        node_id: &str,
    ) -> Result<()> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;

        // Remove from index
        {
            let mut index = index_arc.write().unwrap();
            index.remove(node_id)?;
        }

        let key = IndexKey::new(tenant_id, repo_id, branch, partition);
        self.mark_mutated(&key, &index_arc);

        self.metrics.record_embedding_removed();

        Ok(())
    }

    /// Purge (delete) ONE partition's index completely.
    ///
    /// This removes the in-memory cache entry, the disk file and the sidecar.
    /// Useful when rebuilding with a different shape.
    ///
    /// The sidecar is deleted with the graph file for the same reason it is
    /// shipped with it: an orphan `.hnsw` with no `.hnsw.meta` is read as an
    /// old bincode index and destroys itself on the next load.
    pub fn purge_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<()> {
        let key = IndexKey::new(tenant_id, repo_id, branch, partition);
        let path = key.index_path(self.base_path());

        // Remove from cache
        self.index_cache.invalidate(&key);

        // Remove from dirty set and the re-weigh counter
        self.dirty_indexes.write().unwrap().remove(&key);
        self.mutations_since_weigh.write().unwrap().remove(&key);

        // Delete file if it exists
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete HNSW index file: {}", e))
            })?;
            tracing::info!("Deleted HNSW index file: {}", path.display());
        }

        // Delete metadata sidecar if it exists
        let meta_path = key.meta_path(self.base_path());
        if meta_path.exists() {
            if let Err(e) = std::fs::remove_file(&meta_path) {
                tracing::warn!("Failed to delete HNSW metadata sidecar: {}", e);
            }
        }

        Ok(())
    }

    /// Purge EVERY partition of a branch.
    ///
    /// What "drop this branch's vector index" means once there is more than one
    /// index per branch. Callers that mean one embedding space should name it.
    pub fn purge_branch(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        for partition in self.list_partitions(tenant_id, repo_id, branch)? {
            self.purge_index(tenant_id, repo_id, branch, &partition)?;
        }
        Ok(())
    }

    /// Create a new index for one partition at a specific shape.
    ///
    /// This is useful during rebuild operations when the shape has changed.
    /// Unlike `get_or_load_index`, this will create a NEW index even if one
    /// exists, allowing you to recreate it at a different width, metric or
    /// quantization.
    pub fn create_index_with_spec(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        spec: IndexSpec,
    ) -> Result<()> {
        let key = IndexKey::new(tenant_id, repo_id, branch, partition);

        let index = HnswIndex::with_params(spec.dimensions, spec.metric, spec.params);
        let index_arc = Arc::new(RwLock::new(index));

        // Insert into cache (will replace any existing entry)
        self.index_cache.insert(key.clone(), Arc::clone(&index_arc));

        // Mark as dirty so it gets saved
        self.dirty_indexes.write().unwrap().insert(key.clone());
        self.mutations_since_weigh.write().unwrap().remove(&key);

        tracing::info!(
            index = %key,
            dimensions = spec.dimensions,
            metric = %spec.metric,
            quantization = %spec.quantization(),
            "Created new HNSW index"
        );

        Ok(())
    }

    /// Create a new index for one partition at the shape the resolver reports.
    ///
    /// The rebuild path's entry point: it must not re-derive the width, metric
    /// or quantization itself, because a rebuild that disagrees with
    /// `get_or_load_index` about any of the three produces an index that is
    /// immediately unloadable.
    pub fn create_index_from_config(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<IndexSpec> {
        let key = IndexKey::new(tenant_id, repo_id, branch, partition);
        let spec = self.resolve_spec(&key);
        self.create_index_with_spec(tenant_id, repo_id, branch, partition, spec)?;
        Ok(spec)
    }

    /// Copy EVERY partition of a branch to a new branch.
    ///
    /// This implements Git-like branch semantics by copying the source branch's
    /// indexes to the new branch.
    ///
    /// Per-partition, and the sidecar travels with each graph file — a fork
    /// that copied only `.hnsw` would hand the new branch an index that
    /// destroys itself on first load, which is the same defect the replication
    /// transfer had.
    pub fn copy_for_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
    ) -> Result<()> {
        let partitions = self.list_partitions(tenant_id, repo_id, source_branch)?;

        if partitions.is_empty() {
            tracing::warn!(
                "No HNSW index partitions found for branch {}, skipping copy",
                source_branch
            );
            return Ok(());
        }

        for partition in &partitions {
            let source = IndexKey::new(tenant_id, repo_id, source_branch, partition);
            let target = IndexKey::new(tenant_id, repo_id, target_branch, partition);

            let source_path = source.index_path(self.base_path());
            let target_path = target.index_path(self.base_path());

            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to create directory: {}", e))
                })?;
            }

            // Sidecar first, same ordering rule as the layout migration: a
            // partial copy must never leave a bare `.hnsw` at the target.
            let source_meta = source.meta_path(self.base_path());
            let target_meta = target.meta_path(self.base_path());
            if source_meta.exists() {
                std::fs::copy(&source_meta, &target_meta).map_err(|e| {
                    raisin_error::Error::storage(format!(
                        "Failed to copy HNSW metadata sidecar: {}",
                        e
                    ))
                })?;
            }

            std::fs::copy(&source_path, &target_path).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to copy HNSW index: {}", e))
            })?;
        }

        tracing::info!(
            "Copied {} HNSW index partition(s) from {} to {} (includes all workspaces)",
            partitions.len(),
            source_branch,
            target_branch
        );

        Ok(())
    }
}
