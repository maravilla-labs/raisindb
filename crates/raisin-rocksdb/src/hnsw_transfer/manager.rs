//! HNSW index manager for collecting and loading indexes.
//!
//! Every path here comes from `raisin_hnsw::index_path` — THE path builder.
//! This module used to have its own,
//! `index_base_dir.join(tenant).join(repo).join(format!("{branch}.hnsw"))`,
//! which had already drifted from the engine's `.with_extension("hnsw")` (they
//! disagreed for any branch containing a dot) and which knew nothing about the
//! `.hnsw.meta` sidecar. See `bundle.rs` for what that cost.

use super::bundle;
use super::types::HnswIndexMetadata;
use raisin_error::{Error, Result};
use raisin_hnsw::PartitionId;
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

/// HNSW index manager for transferring vector indexes
pub struct HnswIndexManager {
    /// Base directory for HNSW indexes
    /// Typically: data_dir/hnsw/tenant_id/repo_id/branch/partition.hnsw
    index_base_dir: PathBuf,
}

impl HnswIndexManager {
    /// Create a new HNSW index manager
    ///
    /// # Arguments
    /// * `index_base_dir` - Base directory for HNSW indexes
    pub fn new(index_base_dir: PathBuf) -> Self {
        Self { index_base_dir }
    }

    /// Get the file path for a specific HNSW index partition.
    ///
    /// Delegates to the engine's path builder; there is no second layout.
    pub fn get_index_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> PathBuf {
        raisin_hnsw::index_path(&self.index_base_dir, tenant_id, repo_id, branch, partition)
    }

    /// Collect HNSW index metadata for one partition.
    ///
    /// # Returns
    /// Metadata about the index including size and checksum. Both are over the
    /// BUNDLE (graph + sidecar), because that is what actually crosses the wire
    /// — a size or checksum over the graph alone would not verify the half that
    /// used to go missing.
    pub async fn collect_index_metadata(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<Option<HnswIndexMetadata>> {
        let Some((data, crc32)) = self
            .load_index_data(tenant_id, repo_id, branch, partition)
            .await?
        else {
            return Ok(None);
        };

        let size_bytes = data.len() as u64;
        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            partition = %partition,
            size_bytes,
            crc32,
            "HNSW index metadata collected"
        );

        Ok(Some(HnswIndexMetadata {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            partition: partition.clone(),
            size_bytes,
            crc32,
        }))
    }

    /// Load one partition as a wire-ready bundle: graph file AND sidecar.
    ///
    /// # Returns
    /// The bundled payload and its CRC32.
    pub async fn load_index_data(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<Option<(Vec<u8>, u32)>> {
        let index_path = self.get_index_path(tenant_id, repo_id, branch, partition);

        if !index_path.exists() {
            return Ok(None);
        }

        let meta_path = raisin_hnsw::meta_path(&index_path);
        if !meta_path.exists() {
            // Refuse to ship half an index. Shipping the graph alone is what
            // used to destroy the receiver's copy; there is nothing useful to
            // send here and pretending otherwise is the bug.
            return Err(Error::storage(format!(
                "HNSW index {} has no `.hnsw.meta` sidecar, so it cannot be transferred: \
                 the sidecar holds the node-id mapping, the width and the metric. Run \
                 REBUILD VECTOR INDEX on the source node.",
                index_path.display()
            )));
        }

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            partition = %partition,
            path = %index_path.display(),
            "Loading HNSW index data (graph + sidecar)"
        );

        let graph = fs::read(&index_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to read index file: {}", e)))?;
        let meta = fs::read(&meta_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to read index sidecar: {}", e)))?;

        let data = bundle::pack(&graph, &meta);

        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&data);
        let crc32 = hasher.finalize();

        info!(
            size_mb = data.len() / 1_048_576,
            graph_bytes = graph.len(),
            meta_bytes = meta.len(),
            "HNSW index bundle loaded"
        );

        Ok(Some((data, crc32)))
    }

    /// List every `(tenant, repo, branch, partition)` that has an index on disk.
    ///
    /// The partition is part of the tuple because it is part of the identity: a
    /// branch now holds one index per embedding space, and a transfer that named
    /// only the branch could not say which one it meant.
    pub async fn list_all_indexes(&self) -> Result<Vec<(String, String, String, PartitionId)>> {
        let mut indexes = Vec::new();

        if !self.index_base_dir.exists() {
            return Ok(indexes);
        }

        let mut tenant_entries = fs::read_dir(&self.index_base_dir)
            .await
            .map_err(|e| Error::storage(format!("Failed to read index base directory: {}", e)))?;

        while let Some(tenant_entry) = tenant_entries
            .next_entry()
            .await
            .map_err(|e| Error::storage(format!("Failed to read tenant entry: {}", e)))?
        {
            if !tenant_entry.path().is_dir() {
                continue;
            }
            let tenant_id = tenant_entry.file_name().to_string_lossy().to_string();

            let mut repo_entries = fs::read_dir(tenant_entry.path())
                .await
                .map_err(|e| Error::storage(format!("Failed to read repo directory: {}", e)))?;

            while let Some(repo_entry) = repo_entries
                .next_entry()
                .await
                .map_err(|e| Error::storage(format!("Failed to read repo entry: {}", e)))?
            {
                if !repo_entry.path().is_dir() {
                    continue;
                }
                let repo_id = repo_entry.file_name().to_string_lossy().to_string();

                // Branch directories. A pre-partition deployment has
                // `<branch>.hnsw` FILES here instead; those are migrated into a
                // branch directory on first load by the engine, so they are
                // skipped rather than reported under a partition this node
                // cannot name.
                let mut branch_entries = fs::read_dir(repo_entry.path())
                    .await
                    .map_err(|e| Error::storage(format!("Failed to read repo files: {}", e)))?;

                while let Some(branch_entry) = branch_entries
                    .next_entry()
                    .await
                    .map_err(|e| Error::storage(format!("Failed to read branch entry: {}", e)))?
                {
                    let branch_path = branch_entry.path();
                    if !branch_path.is_dir() {
                        continue;
                    }
                    let branch = branch_entry.file_name().to_string_lossy().to_string();

                    for partition in raisin_hnsw::list_partitions_in(&branch_path) {
                        indexes.push((
                            tenant_id.clone(),
                            repo_id.clone(),
                            branch.clone(),
                            partition,
                        ));
                    }
                }
            }
        }

        Ok(indexes)
    }
}
