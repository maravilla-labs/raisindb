// SPDX-License-Identifier: BSL-1.1

//! Index mutation operations for the HNSW indexing engine.
//!
//! Provides methods for adding/removing embeddings, purging indexes,
//! creating indexes with specific dimensions, and copying indexes for branch operations.

use crate::index::HnswIndex;
use crate::types::DistanceMetric;
use raisin_error::Result;
use raisin_hlc::HLC;
use std::sync::{Arc, RwLock};

use super::HnswIndexingEngine;

impl HnswIndexingEngine {
    /// Add an embedding to the index.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `workspace_id` - Workspace identifier (stored as metadata, not in key)
    /// * `node_id` - Node identifier
    /// * `revision` - Revision (full HLC with timestamp and counter)
    /// * `embedding` - Embedding vector
    pub fn add_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: HLC,
        embedding: Vec<f32>,
    ) -> Result<()> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch)?;

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

        // Mark as dirty
        let key = self.make_key(tenant_id, repo_id, branch);
        self.dirty_indexes.write().unwrap().insert(key);

        Ok(())
    }

    /// Remove an embedding from the index.
    ///
    /// Note: workspace_id is no longer needed as parameter since all workspaces
    /// are in the same index. The node_id alone is sufficient for removal.
    pub fn remove_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
    ) -> Result<()> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch)?;

        // Remove from index
        {
            let mut index = index_arc.write().unwrap();
            index.remove(node_id)?;
        }

        // Mark as dirty
        let key = self.make_key(tenant_id, repo_id, branch);
        self.dirty_indexes.write().unwrap().insert(key);

        Ok(())
    }

    /// Purge (delete) an HNSW index completely.
    ///
    /// This removes both the in-memory cache entry and the disk file.
    /// Useful when rebuilding with different dimensions.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `_workspace_id` - Not used (kept for API compatibility)
    pub fn purge_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        _workspace_id: &str,
    ) -> Result<()> {
        let key = self.make_key(tenant_id, repo_id, branch);
        let path = self.get_index_path(&key);

        // Remove from cache
        self.index_cache.invalidate(&key);

        // Remove from dirty set
        self.dirty_indexes.write().unwrap().remove(&key);

        // Delete file if it exists
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to delete HNSW index file: {}", e))
            })?;
            tracing::info!("Deleted HNSW index file: {}", path.display());
        }

        // Delete metadata sidecar if it exists
        let meta_path = crate::persistence::meta_path_for(&path);
        if meta_path.exists() {
            if let Err(e) = std::fs::remove_file(&meta_path) {
                tracing::warn!("Failed to delete HNSW metadata sidecar: {}", e);
            }
        }

        Ok(())
    }

    /// Create a new index with specific dimensions and the engine's default metric.
    ///
    /// This is useful during rebuild operations when dimensions have changed.
    /// Unlike `get_or_load_index`, this will create a NEW index even if one exists,
    /// allowing you to recreate with different dimensions.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `dimensions` - Vector dimensionality for the new index
    pub fn create_index_with_dimensions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        dimensions: usize,
    ) -> Result<()> {
        self.create_index_with_dimensions_and_metric(
            tenant_id,
            repo_id,
            branch,
            dimensions,
            self.distance_metric,
        )
    }

    /// Create a new index with specific dimensions and a specific distance metric.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `dimensions` - Vector dimensionality for the new index
    /// * `metric` - Distance metric for the new index
    pub fn create_index_with_dimensions_and_metric(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        dimensions: usize,
        metric: DistanceMetric,
    ) -> Result<()> {
        let key = self.make_key(tenant_id, repo_id, branch);

        // Create new index with specified dimensions and metric
        let index = HnswIndex::with_metric(dimensions, metric);
        let index_arc = Arc::new(RwLock::new(index));

        // Insert into cache (will replace any existing entry)
        self.index_cache.insert(key.clone(), index_arc);

        // Mark as dirty so it gets saved
        self.dirty_indexes.write().unwrap().insert(key);

        tracing::info!(
            "Created new HNSW index for {}/{}/{} with {} dimensions and {} metric",
            tenant_id,
            repo_id,
            branch,
            dimensions,
            metric
        );

        Ok(())
    }

    /// Copy index for branch creation.
    ///
    /// This implements Git-like branch semantics by copying the source
    /// branch's index to the new branch.
    ///
    /// Note: Since indexes now contain ALL workspaces, we only need to
    /// copy once per tenant/repo/branch (not per workspace).
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `source_branch` - Source branch to copy from
    /// * `target_branch` - New branch to create
    pub fn copy_for_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
    ) -> Result<()> {
        let source_path = self.get_index_path(&self.make_key(tenant_id, repo_id, source_branch));

        let target_path = self.get_index_path(&self.make_key(tenant_id, repo_id, target_branch));

        if !source_path.exists() {
            tracing::warn!(
                "Source HNSW index not found for branch {}, skipping copy",
                source_branch
            );
            return Ok(());
        }

        // Ensure target directory exists
        if let Some(parent) = target_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to create directory: {}", e))
            })?;
        }

        // Copy index file
        std::fs::copy(&source_path, &target_path).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to copy HNSW index: {}", e))
        })?;

        // Copy metadata sidecar if it exists
        let source_meta = crate::persistence::meta_path_for(&source_path);
        let target_meta = crate::persistence::meta_path_for(&target_path);
        if source_meta.exists() {
            std::fs::copy(&source_meta, &target_meta).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to copy HNSW metadata sidecar: {}", e))
            })?;
        }

        tracing::info!(
            "Copied HNSW index from {} to {} (includes all workspaces)",
            source_branch,
            target_branch
        );

        Ok(())
    }
}
