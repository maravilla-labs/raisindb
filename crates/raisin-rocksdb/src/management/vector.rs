//! Management operations for HNSW vector indexes.
//!
//! This module provides administrative operations for HNSW indexes including:
//! - Rebuilding indexes from stored embeddings
//! - Verifying index consistency
//! - Health monitoring
//! - Optimization

use crate::{RocksDBEmbeddingStorage, TenantEmbeddingConfigRepository};
use raisin_embeddings::{storage::TenantEmbeddingConfigStore, EmbeddingStorage};
use raisin_error::{Error, Result};
use raisin_hnsw::HnswIndexingEngine;
use raisin_storage::jobs::global_registry;
use raisin_storage::{IndexHealth, IndexStatus, JobId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Statistics from a vector index rebuild operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildStats {
    pub items_processed: usize,
    pub errors: usize,
    pub segments_merged: usize,
    pub duration_ms: u64,
}

/// Report from a vector index verification operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub status: IndexStatus,
    pub embeddings_in_rocksdb: usize,
    pub embeddings_in_hnsw: usize,
    pub mismatches: usize,
    pub dimension_mismatches: Vec<DimensionMismatch>,
}

/// Dimension mismatch details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionMismatch {
    pub node_id: String,
    pub expected_dims: usize,
    pub actual_dims: usize,
}

/// Management operations for HNSW vector indexes
pub struct HnswManagement {
    /// HNSW indexing engine
    hnsw_engine: Arc<HnswIndexingEngine>,
    /// Embedding storage for accessing stored embeddings
    embedding_storage: Arc<RocksDBEmbeddingStorage>,
    /// Tenant config repository for getting embedding dimensions
    config_repo: TenantEmbeddingConfigRepository,
}

impl HnswManagement {
    /// Create new HNSW management instance
    pub fn new(
        hnsw_engine: Arc<HnswIndexingEngine>,
        embedding_storage: Arc<RocksDBEmbeddingStorage>,
        config_repo: TenantEmbeddingConfigRepository,
    ) -> Self {
        Self {
            hnsw_engine,
            embedding_storage,
            config_repo,
        }
    }

    /// Rebuild vector index from stored embeddings in RocksDB
    ///
    /// This operation:
    /// 1. Gets the correct dimensions from TenantEmbeddingConfig
    /// 2. Deletes the existing HNSW index file
    /// 3. Recreates the index with the correct dimensions
    /// 4. Re-adds all embeddings from the embeddings CF
    /// 5. Reports progress via JobRegistry
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `job_id` - Optional job ID for progress reporting
    pub async fn rebuild_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        job_id: Option<JobId>,
    ) -> Result<RebuildStats> {
        let start_time = std::time::Instant::now();

        tracing::info!(
            "Starting vector index rebuild for {}/{}/{}",
            tenant_id,
            repo_id,
            branch
        );

        // Get tenant embedding config to determine correct dimensions
        let config = self
            .config_repo
            .get_config(tenant_id)
            .map_err(|e| Error::storage(format!("Failed to get config: {}", e)))?
            .ok_or_else(|| {
                Error::storage(format!(
                    "No embedding config found for tenant '{}'",
                    tenant_id
                ))
            })?;

        if !config.enabled {
            return Err(Error::storage(format!(
                "Embeddings are disabled for tenant '{}'",
                tenant_id
            )));
        }

        let dimensions = config.dimensions;
        tracing::info!("Using dimensions: {} from tenant config", dimensions);

        // Purge existing index (dimension might have changed)
        tracing::info!("Purging existing HNSW index...");
        self.hnsw_engine
            .purge_index(tenant_id, repo_id, branch, "staff")
            .map_err(|e| Error::storage(format!("Failed to purge index: {}", e)))?;

        // Create new index with correct dimensions
        tracing::info!("Creating new HNSW index with {} dimensions...", dimensions);
        self.hnsw_engine
            .create_index_with_dimensions(tenant_id, repo_id, branch, dimensions)
            .map_err(|e| Error::storage(format!("Failed to create index: {}", e)))?;

        // Get all embeddings from RocksDB
        tracing::info!("Listing embeddings from RocksDB...");
        let embeddings_list = self
            .embedding_storage
            .list_embeddings(tenant_id, repo_id, branch, "staff")?;

        let total_embeddings = embeddings_list.len();
        tracing::info!("Found {} embeddings to rebuild", total_embeddings);

        if total_embeddings == 0 {
            tracing::warn!("No embeddings found, nothing to rebuild");
            return Ok(RebuildStats {
                items_processed: 0,
                errors: 0,
                segments_merged: 0,
                duration_ms: start_time.elapsed().as_millis() as u64,
            });
        }

        // Report initial progress
        if let Some(ref jid) = job_id {
            let _ = global_registry().update_progress(jid, 0.0).await;
        }

        let mut items_processed = 0;
        let mut errors = 0;

        // Re-add all embeddings to HNSW
        for (idx, (node_id, revision)) in embeddings_list.iter().enumerate() {
            // Fetch embedding from RocksDB
            match self.embedding_storage.get_embedding(
                tenant_id,
                repo_id,
                branch,
                "staff",
                node_id,
                Some(revision),
            ) {
                Ok(Some(embedding_data)) => {
                    // Verify dimensions match
                    if embedding_data.vector.len() != dimensions {
                        tracing::warn!(
                            "Dimension mismatch for {}: expected {}, got {} - skipping",
                            node_id,
                            dimensions,
                            embedding_data.vector.len()
                        );
                        errors += 1;
                        continue;
                    }

                    // Add to HNSW index (using full HLC)
                    if let Err(e) = self.hnsw_engine.add_embedding(
                        tenant_id,
                        repo_id,
                        branch,
                        "staff",
                        node_id,
                        *revision,
                        embedding_data.vector,
                    ) {
                        tracing::error!("Failed to add embedding for {}: {}", node_id, e);
                        errors += 1;
                        continue;
                    }

                    items_processed += 1;

                    // Report progress every 100 items or on last item
                    if idx % 100 == 0 || idx == total_embeddings - 1 {
                        let progress = (idx as f32 + 1.0) / total_embeddings as f32;

                        if let Some(ref jid) = job_id {
                            let _ = global_registry().update_progress(jid, progress).await;
                        }

                        tracing::debug!(
                            "Rebuild progress: {}/{} ({:.1}%)",
                            idx + 1,
                            total_embeddings,
                            progress * 100.0
                        );
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        "Embedding not found for node {}, revision {}",
                        node_id,
                        revision
                    );
                    errors += 1;
                }
                Err(e) => {
                    tracing::error!("Failed to fetch embedding for {}: {}", node_id, e);
                    errors += 1;
                }
            }
        }

        // Force a snapshot to persist the rebuilt index
        tracing::info!("Saving rebuilt HNSW index to disk...");
        if let Err(e) = self.hnsw_engine.snapshot_dirty_indexes() {
            tracing::error!("Failed to snapshot HNSW index: {}", e);
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;

        tracing::info!(
            "Vector index rebuild completed: {} items processed, {} errors, {}ms",
            items_processed,
            errors,
            duration_ms
        );

        Ok(RebuildStats {
            items_processed,
            errors,
            segments_merged: 0, // HNSW doesn't have segment merging like Tantivy
            duration_ms,
        })
    }

    /// Verify vector index consistency
    ///
    /// Checks that all embeddings in RocksDB have matching dimensions.
    pub async fn verify_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<VerificationReport> {
        tracing::info!(
            "Verifying vector index for {}/{}/{}",
            tenant_id,
            repo_id,
            branch
        );

        // Get tenant config for expected dimensions
        let config = self
            .config_repo
            .get_config(tenant_id)
            .map_err(|e| Error::storage(format!("Failed to get config: {}", e)))?
            .ok_or_else(|| Error::storage("Tenant config not found".to_string()))?;

        let expected_dims = config.dimensions;

        // List embeddings from RocksDB
        let embeddings_list = self
            .embedding_storage
            .list_embeddings(tenant_id, repo_id, branch, "staff")?;

        let embeddings_in_rocksdb = embeddings_list.len();
        let mut mismatches = 0;
        let mut dimension_mismatches = Vec::new();

        // Check each embedding
        for (node_id, revision) in embeddings_list {
            match self.embedding_storage.get_embedding(
                tenant_id,
                repo_id,
                branch,
                "staff",
                &node_id,
                Some(&revision),
            ) {
                Ok(Some(data)) => {
                    if data.vector.len() != expected_dims {
                        dimension_mismatches.push(DimensionMismatch {
                            node_id: node_id.clone(),
                            expected_dims,
                            actual_dims: data.vector.len(),
                        });
                        mismatches += 1;
                    }
                }
                Ok(None) => {
                    mismatches += 1;
                }
                Err(_) => {
                    mismatches += 1;
                }
            }
        }

        let status = if mismatches == 0 {
            IndexStatus::Healthy
        } else if mismatches < embeddings_in_rocksdb / 10 {
            IndexStatus::Degraded
        } else {
            IndexStatus::Corrupted
        };

        Ok(VerificationReport {
            status,
            embeddings_in_rocksdb,
            embeddings_in_hnsw: embeddings_in_rocksdb - mismatches,
            mismatches,
            dimension_mismatches,
        })
    }

    /// Get health status of vector index
    pub async fn get_health(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<IndexHealth> {
        // Get config for dimensions
        let config = self
            .config_repo
            .get_config(tenant_id)
            .map_err(|e| Error::storage(format!("Failed to get config: {}", e)))?;

        let dimensions = config.as_ref().map(|c| c.dimensions).unwrap_or(0);
        let _enabled = config.as_ref().map(|c| c.enabled).unwrap_or(false);

        // Count embeddings
        let embedding_count = self
            .embedding_storage
            .list_embeddings(tenant_id, repo_id, branch, "staff")?
            .len();

        Ok(IndexHealth {
            index_type: format!("HNSW ({}d)", dimensions),
            memory_usage_bytes: 0, // TODO: Get from HNSW engine
            disk_usage_bytes: 0,   // TODO: Get from HNSW engine
            entry_count: embedding_count as u64,
            cache_hit_rate: None,
            last_optimized: None, // HNSW doesn't need optimization
        })
    }

    /// Optimize vector index (currently a no-op for HNSW)
    ///
    /// HNSW doesn't need optimization like Tantivy's segment merging.
    /// This is here for API completeness.
    pub async fn optimize_index(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
    ) -> Result<RebuildStats> {
        Ok(RebuildStats {
            items_processed: 0,
            errors: 0,
            segments_merged: 0,
            duration_ms: 0,
        })
    }

    /// Purge vector index completely
    pub async fn purge_index(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        tracing::warn!(
            "Purging vector index for {}/{}/{}",
            tenant_id,
            repo_id,
            branch
        );

        self.hnsw_engine
            .purge_index(tenant_id, repo_id, branch, "staff")
            .map_err(|e| Error::storage(format!("Failed to purge index: {}", e)))?;

        Ok(())
    }
}
