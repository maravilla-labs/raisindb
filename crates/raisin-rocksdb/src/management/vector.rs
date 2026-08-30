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
use raisin_hnsw::{HnswIndexingEngine, PartitionId};
use raisin_storage::jobs::global_registry;
use raisin_storage::{IndexHealth, IndexStatus, JobId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Statistics from a vector index rebuild operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuildStats {
    /// Embeddings actually ADDED to the index.
    ///
    /// Never the number listed from storage — a rebuild that skipped every
    /// vector must not report a count that looks like it worked.
    pub items_processed: usize,
    /// Embeddings that were listed but not indexed: width mismatch, missing
    /// row, or a failed insert.
    pub errors: usize,
    pub segments_merged: usize,
    pub duration_ms: u64,
    /// The workspaces this rebuild covered.
    ///
    /// `#[serde(default)]` so a stats blob written by an older binary still
    /// deserializes out of the job registry.
    #[serde(default)]
    pub workspaces: Vec<String>,
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
///
/// # One rebuild, every workspace
///
/// Embeddings are stored per WORKSPACE: the embedding job writes under
/// `context.workspace_id`, i.e. whichever workspace the node it embedded lives
/// in. Administrative operations therefore have to cover the whole
/// `{tenant}/{repo}/{branch}`, and they discover the workspace set with
/// `EmbeddingStorage::list_workspaces` rather than naming one.
///
/// Both surfaces that rebuild — the HTTP management endpoint and SQL's
/// `REBUILD VECTOR INDEX` — go through the methods here. There used to be two
/// separate loops that had drifted apart in three ways at once: each hardcoded
/// a *different* workspace literal (`"staff"` here, `"default"` in SQL), only
/// this one checked vector width before inserting, and only this one reported
/// what it had actually added. Keep it one implementation.
pub struct HnswManagement {
    /// HNSW indexing engine
    hnsw_engine: Arc<HnswIndexingEngine>,
    /// Embedding storage for accessing stored embeddings
    ///
    /// Held as the TRAIT, not `RocksDBEmbeddingStorage`, so the SQL execution
    /// layer can build one of these out of the `Arc<dyn EmbeddingStorage>` it
    /// already carries instead of keeping a second rebuild loop of its own.
    embedding_storage: Arc<dyn EmbeddingStorage>,
    /// Tenant config store for getting embedding dimensions
    config_store: Arc<dyn TenantEmbeddingConfigStore>,
}

impl HnswManagement {
    /// Create new HNSW management instance from the concrete RocksDB types.
    pub fn new(
        hnsw_engine: Arc<HnswIndexingEngine>,
        embedding_storage: Arc<RocksDBEmbeddingStorage>,
        config_repo: TenantEmbeddingConfigRepository,
    ) -> Self {
        Self::from_stores(hnsw_engine, embedding_storage, Arc::new(config_repo))
    }

    /// Create new HNSW management instance from trait objects.
    ///
    /// This is what the SQL execution layer uses: it holds
    /// `Arc<dyn EmbeddingStorage>` and `Arc<dyn TenantEmbeddingConfigStore>`,
    /// never the concrete RocksDB types.
    pub fn from_stores(
        hnsw_engine: Arc<HnswIndexingEngine>,
        embedding_storage: Arc<dyn EmbeddingStorage>,
        config_store: Arc<dyn TenantEmbeddingConfigStore>,
    ) -> Self {
        Self {
            hnsw_engine,
            embedding_storage,
            config_store,
        }
    }

    /// Every workspace under this branch that holds at least one embedding.
    fn workspaces(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Vec<String>> {
        self.embedding_storage
            .list_workspaces(tenant_id, repo_id, branch)
    }

    /// Rebuild vector index from stored embeddings in RocksDB
    ///
    /// This operation, for EVERY workspace under the branch:
    /// 1. Gets the correct dimensions from TenantEmbeddingConfig
    /// 2. Deletes the existing HNSW index
    /// 3. Recreates the index with the correct dimensions
    /// 4. Re-adds all embeddings from the embeddings CF, skipping any whose
    ///    width does not match the configured one
    /// 5. Reports progress via JobRegistry
    ///
    /// `RebuildStats::items_processed` counts embeddings actually ADDED to the
    /// index — never the number merely listed from storage.
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
            .config_store
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

        // WHICH index. Resolved through the engine's one spec resolver, which is
        // the same read the write path and `get_or_load_index` use — a rebuild
        // that named a different partition would recreate an index nothing ever
        // queries and leave the real one stale.
        let partition = self
            .hnsw_engine
            .default_text_partition(tenant_id, repo_id, branch)
            .ok_or_else(|| {
                Error::storage(format!(
                    "Cannot resolve the vector index partition for tenant '{}': its embedding \
                     provider does not resolve, so there is no embedder identity to rebuild under",
                    tenant_id
                ))
            })?;

        tracing::info!(
            "Using dimensions: {} from tenant config, partition {}",
            dimensions,
            partition
        );

        let workspaces = self.workspaces(tenant_id, repo_id, branch)?;
        tracing::info!(
            "Rebuilding {} workspace(s): {:?}",
            workspaces.len(),
            workspaces
        );

        // Collect the work up front so progress is reported against the real
        // total across all workspaces, not restarted per workspace.
        //
        // The unit is a CHUNK in THIS partition, not a node. `list_embeddings`
        // collapses a document's chunks into one row and says nothing about
        // which embedding space wrote them, so driving the rebuild off it
        // re-indexed one arbitrary chunk per node and dropped the rest —
        // turning `REBUILD VECTOR INDEX` into a data-loss command on any
        // chunked corpus, with `VERIFY` afterwards reporting the wreckage as
        // "consistent". `list_index_entries` yields exactly the rows the index
        // holds, so listed == indexable and the two counts are commensurable.
        let want_hash = partition.embedder_hash();
        let want_kind = partition.kind_char();
        let mut work: Vec<(String, Vec<raisin_embeddings::StoredIndexEntry>)> = Vec::new();
        for workspace in &workspaces {
            let list: Vec<raisin_embeddings::StoredIndexEntry> = self
                .embedding_storage
                .list_index_entries(tenant_id, repo_id, branch, workspace)?
                .into_iter()
                .filter(|e| {
                    Some(e.embedder_hash.as_str()) == want_hash && Some(e.kind) == want_kind
                })
                .collect();
            work.push((workspace.clone(), list));
        }

        let total_embeddings: usize = work.iter().map(|(_, l)| l.len()).sum();
        tracing::info!("Found {} embeddings to rebuild", total_embeddings);

        // Purge and recreate THIS partition's index at the configured shape.
        // Happens even when there is nothing to re-add: a stale index at the
        // wrong width must not survive a rebuild.
        //
        // Once per branch, not once per workspace: an index has covered every
        // workspace since workspace filtering moved inside the graph walk, so the
        // old per-workspace loop purged and recreated the SAME index N times and
        // the last iteration won. It is also `create_index_from_config` rather
        // than a locally re-derived width, so the rebuild cannot disagree with
        // `get_or_load_index` about the metric or the quantization — a
        // disagreement there produces an index that is unloadable the moment it
        // is written.
        self.hnsw_engine
            .purge_index(tenant_id, repo_id, branch, &partition)
            .map_err(|e| Error::storage(format!("Failed to purge index: {}", e)))?;
        let spec = self
            .hnsw_engine
            .create_index_from_config(tenant_id, repo_id, branch, &partition)
            .map_err(|e| Error::storage(format!("Failed to create index: {}", e)))?;
        tracing::info!(
            partition = %partition,
            dimensions = spec.dimensions,
            metric = %spec.metric,
            quantization = %spec.quantization(),
            "Recreated HNSW index for rebuild"
        );

        if total_embeddings == 0 {
            tracing::warn!("No embeddings found, nothing to rebuild");
            return Ok(RebuildStats {
                items_processed: 0,
                errors: 0,
                segments_merged: 0,
                duration_ms: start_time.elapsed().as_millis() as u64,
                workspaces,
            });
        }

        if let Some(ref jid) = job_id {
            let _ = global_registry().update_progress(jid, 0.0).await;
        }

        let mut items_processed = 0;
        let mut errors = 0;
        let mut seen = 0usize;

        for (workspace, entries) in &work {
            for entry in entries {
                seen += 1;

                // Addressed exactly — embedder, kind, source AND chunk. The old
                // `get_embedding` took a bare node id, so it answered with
                // whichever embedding space sorted first and always with chunk 0.
                match self.embedding_storage.get_source_chunk(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &entry.embedder_hash,
                    entry.kind,
                    &entry.source_id,
                    entry.chunk_index,
                    Some(&entry.revision),
                ) {
                    Ok(Some(embedding_data)) => {
                        // Verify dimensions match
                        if embedding_data.vector.len() != dimensions {
                            tracing::warn!(
                                "Dimension mismatch for {} chunk {} in workspace {}: expected {}, got {} - skipping",
                                entry.source_id,
                                entry.chunk_index,
                                workspace,
                                dimensions,
                                embedding_data.vector.len()
                            );
                            errors += 1;
                            continue;
                        }

                        // The id the WRITER used, derived by the writer's own
                        // function from the row's stored `total_chunks`. A
                        // locally re-derived id would be a second spelling of
                        // the grammar, and a rebuilt index whose ids disagree
                        // with the live path's is a search that finds nothing
                        // and reports no fault.
                        let index_id = raisin_hnsw::index_id_for_stored(
                            &entry.source_id,
                            embedding_data.chunk_index,
                            embedding_data.total_chunks,
                        );

                        if let Err(e) = self.hnsw_engine.add_embedding(
                            tenant_id,
                            repo_id,
                            branch,
                            &partition,
                            workspace,
                            &index_id,
                            entry.revision,
                            embedding_data.vector,
                        ) {
                            tracing::error!("Failed to add embedding for {}: {}", index_id, e);
                            errors += 1;
                            continue;
                        }

                        items_processed += 1;
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "Embedding not found for {} chunk {}, revision {}",
                            entry.source_id,
                            entry.chunk_index,
                            entry.revision
                        );
                        errors += 1;
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to fetch embedding for {} chunk {}: {}",
                            entry.source_id,
                            entry.chunk_index,
                            e
                        );
                        errors += 1;
                    }
                }

                if seen % 100 == 0 || seen == total_embeddings {
                    let progress = seen as f32 / total_embeddings as f32;
                    if let Some(ref jid) = job_id {
                        let _ = global_registry().update_progress(jid, progress).await;
                    }
                    tracing::debug!(
                        "Rebuild progress: {}/{} ({:.1}%)",
                        seen,
                        total_embeddings,
                        progress * 100.0
                    );
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
            "Vector index rebuild completed: {} items indexed, {} skipped, {}ms",
            items_processed,
            errors,
            duration_ms
        );

        Ok(RebuildStats {
            items_processed,
            errors,
            segments_merged: 0, // HNSW doesn't have segment merging like Tantivy
            duration_ms,
            workspaces,
        })
    }

    /// Verify vector index consistency
    ///
    /// Checks that all embeddings in RocksDB have matching dimensions, across
    /// every workspace under the branch.
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
            .config_store
            .get_config(tenant_id)
            .map_err(|e| Error::storage(format!("Failed to get config: {}", e)))?
            .ok_or_else(|| Error::storage("Tenant config not found".to_string()))?;

        let expected_dims = config.dimensions;

        let mut embeddings_in_rocksdb = 0;
        let mut mismatches = 0;
        let mut dimension_mismatches = Vec::new();

        // Per CHUNK, like the rebuild and the SQL `VERIFY`. Per node, this
        // checked the width of chunk 0 and declared the other twenty-two
        // chunks of a long document healthy without reading them.
        for workspace in self.workspaces(tenant_id, repo_id, branch)? {
            let entries = self
                .embedding_storage
                .list_index_entries(tenant_id, repo_id, branch, &workspace)?;

            embeddings_in_rocksdb += entries.len();

            for entry in entries {
                match self.embedding_storage.get_source_chunk(
                    tenant_id,
                    repo_id,
                    branch,
                    &workspace,
                    &entry.embedder_hash,
                    entry.kind,
                    &entry.source_id,
                    entry.chunk_index,
                    Some(&entry.revision),
                ) {
                    Ok(Some(data)) => {
                        if data.vector.len() != expected_dims {
                            dimension_mismatches.push(DimensionMismatch {
                                node_id: raisin_hnsw::index_id_for_stored(
                                    &entry.source_id,
                                    data.chunk_index,
                                    data.total_chunks,
                                ),
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

    /// Get health status of vector index, across every workspace on the branch.
    pub async fn get_health(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<IndexHealth> {
        // Get config for dimensions
        let config = self
            .config_store
            .get_config(tenant_id)
            .map_err(|e| Error::storage(format!("Failed to get config: {}", e)))?;

        let dimensions = config.as_ref().map(|c| c.dimensions).unwrap_or(0);
        let _enabled = config.as_ref().map(|c| c.enabled).unwrap_or(false);

        // Count VECTORS, so `entry_count` is comparable with what the index
        // itself reports. A per-node count made a chunked corpus look like it
        // had lost most of its index.
        let mut embedding_count = 0;
        for workspace in self.workspaces(tenant_id, repo_id, branch)? {
            embedding_count += self
                .embedding_storage
                .list_index_entries(tenant_id, repo_id, branch, &workspace)?
                .len();
        }

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
            workspaces: Vec::new(),
        })
    }

    /// Purge vector index completely, for every workspace on the branch.
    pub async fn purge_index(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        tracing::warn!(
            "Purging vector index for {}/{}/{}",
            tenant_id,
            repo_id,
            branch
        );

        // EVERY partition. "Purge the vector index for this branch" means all of
        // its embedding spaces; leaving a previous model's index behind would
        // make a purge look complete while search kept answering from it.
        self.hnsw_engine
            .purge_branch(tenant_id, repo_id, branch)
            .map_err(|e| Error::storage(format!("Failed to purge index: {}", e)))?;

        Ok(())
    }

    /// Per-partition index health for `SHOW VECTOR INDEX HEALTH`.
    ///
    /// One row per embedding space on the branch, read from disk so a partition
    /// that exists but has never been loaded still appears. An operator cannot
    /// rebuild a partition they cannot see.
    pub fn partition_stats(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Vec<(PartitionId, raisin_hnsw::IndexStats)>> {
        let mut out = Vec::new();
        for partition in self
            .hnsw_engine
            .list_partitions(tenant_id, repo_id, branch)
            .map_err(|e| Error::storage(format!("Failed to list index partitions: {}", e)))?
        {
            match self
                .hnsw_engine
                .stats(tenant_id, repo_id, branch, &partition)
            {
                Ok(stats) => out.push((partition, stats)),
                Err(e) => tracing::warn!(
                    partition = %partition,
                    "Vector index partition could not be opened: {e}"
                ),
            }
        }
        Ok(out)
    }

    /// Rebuild ONE named partition, leaving the branch's other embedding spaces
    /// alone.
    ///
    /// `rebuild_index` rebuilds the tenant's configured text partition, which is
    /// the whole story today. This is the entry point for the day it is not —
    /// and for an operator who has seen one partition go stale in
    /// `partition_stats` and does not want to re-encode the others.
    pub async fn rebuild_partition(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        job_id: Option<JobId>,
    ) -> Result<RebuildStats> {
        let configured = self
            .hnsw_engine
            .default_text_partition(tenant_id, repo_id, branch);
        if configured.as_ref() != Some(partition) {
            // Rebuilding a partition whose model the tenant is no longer on
            // would re-encode nothing useful: `cf::EMBEDDINGS` rows for that
            // embedder are not what `list_embeddings` returns, so the rebuild
            // would fill the old partition with the NEW model's vectors — an
            // index of the wrong space that every distance would rank
            // confidently and wrongly.
            return Err(Error::storage(format!(
                "Partition '{}' is not the tenant's configured embedding partition{}. \
                 Rebuilding it would fill it with vectors from a different model. Point the \
                 tenant's embedding config at that model first, or purge the partition.",
                partition,
                configured
                    .map(|p| format!(" (which is '{p}')"))
                    .unwrap_or_default()
            )));
        }
        self.rebuild_index(tenant_id, repo_id, branch, job_id).await
    }
}
