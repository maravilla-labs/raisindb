//! HNSW index receiver for ingesting indexes from other nodes.
//!
//! Two rules, both learned from the bug in `bundle.rs`:
//!
//! 1. **Nothing is written to the index directory until it has been proved
//!    loadable.** The old receiver renamed the local index to `.backup` and then
//!    moved an unverified payload into its place — so a peer running an older
//!    build did not merely fail to deliver an index, it destroyed the one that
//!    was already there.
//! 2. **Paths come from `raisin_hnsw::index_path`**, not from a `format!` in
//!    this file. This module's private copy had already drifted from the
//!    engine's for any branch containing a dot.

use super::bundle;
use raisin_error::{Error, Result};
use raisin_hnsw::PartitionId;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

/// HNSW index receiver for ingesting indexes from other nodes
pub struct HnswIndexReceiver {
    /// Base directory for HNSW indexes
    index_base_dir: PathBuf,

    /// Staging directory for incoming indexes
    staging_dir: PathBuf,
}

impl HnswIndexReceiver {
    /// Create a new HNSW index receiver
    pub fn new(index_base_dir: PathBuf, staging_dir: PathBuf) -> Self {
        Self {
            index_base_dir,
            staging_dir,
        }
    }

    /// Where a partition's incoming graph file is staged.
    fn staging_path(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> PathBuf {
        // Slashes cannot appear in a staging FILE name, and a branch may contain
        // them, so the components are joined with `~`. Collisions between
        // different branches are irrelevant here — a staging file is consumed
        // immediately and `abort_receive` deletes it by the same construction.
        self.staging_dir.join(format!(
            "hnsw~{}~{}~{}~{}.hnsw",
            tenant_id,
            repo_id,
            branch.replace(['/', '\\'], "_"),
            partition
        ))
    }

    /// Receive, verify and STAGE one partition's index bundle.
    ///
    /// Verification is three-layered and all of it happens before anything is
    /// written outside the staging directory:
    ///
    /// * the CRC32 covers the whole bundle, so a truncated sidecar is caught;
    /// * [`bundle::unpack`] refuses a payload that is not a bundle — i.e. a bare
    ///   graph file from a peer on an older build, which is precisely the shape
    ///   that used to be ingested and then destroy itself;
    /// * the staged pair is opened with the real loader, so a payload that is
    ///   bundled but not actually an index never reaches the index directory.
    ///
    /// # Returns
    /// Path to the staged `.hnsw` file (its `.hnsw.meta` sits beside it).
    pub async fn receive_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
        data: Vec<u8>,
        expected_crc32: u32,
    ) -> Result<PathBuf> {
        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            partition = %partition,
            size_mb = data.len() / 1_048_576,
            "Receiving HNSW index"
        );

        // Calculate CRC32 of received data
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&data);
        let calculated_crc32 = hasher.finalize();

        if calculated_crc32 != expected_crc32 {
            return Err(Error::storage(format!(
                "HNSW index checksum mismatch: expected {}, got {}",
                expected_crc32, calculated_crc32
            )));
        }

        let (graph, meta) = bundle::unpack(&data)?;

        let staging_path = self.staging_path(tenant_id, repo_id, branch, partition);
        if let Some(parent) = staging_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::storage(format!("Failed to create staging dir: {}", e)))?;
        }
        let staging_meta = raisin_hnsw::meta_path(&staging_path);

        // Sidecar first, everywhere: a `.hnsw` that briefly exists without its
        // `.hnsw.meta` is the shape the loader mistakes for a bincode index.
        fs::write(&staging_meta, meta)
            .await
            .map_err(|e| Error::storage(format!("Failed to write staging sidecar: {}", e)))?;
        fs::write(&staging_path, graph)
            .await
            .map_err(|e| Error::storage(format!("Failed to write staging file: {}", e)))?;

        // Prove it opens BEFORE it is allowed anywhere near the live directory.
        let probe = staging_path.clone();
        let opened: Result<(usize, usize)> = tokio::task::spawn_blocking(move || {
            raisin_hnsw::HnswIndex::view_from_file(&probe).map(|i| (i.len(), i.dimensions()))
        })
        .await
        .map_err(|e| Error::storage(format!("Index verification task failed: {}", e)))?;

        match opened {
            Ok((count, dimensions)) => info!(
                partition = %partition,
                vectors = count,
                dimensions,
                "Received HNSW index verified: it loads and holds vectors"
            ),
            Err(e) => {
                let _ = fs::remove_file(&staging_path).await;
                let _ = fs::remove_file(&staging_meta).await;
                return Err(Error::storage(format!(
                    "Received HNSW index does not load, so it will NOT be ingested (the local \
                     index is untouched): {e}"
                )));
            }
        }

        info!(
            staging_path = %staging_path.display(),
            "HNSW index received and verified"
        );

        Ok(staging_path)
    }

    /// Ingest a staged index by moving BOTH files to their final location.
    ///
    /// # Arguments
    /// * `staging_path` - Path to the staged `.hnsw` (verified by `receive_index`)
    pub async fn ingest_index(
        &self,
        staging_path: &Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<()> {
        let target_path =
            raisin_hnsw::index_path(&self.index_base_dir, tenant_id, repo_id, branch, partition);
        let target_meta = raisin_hnsw::meta_path(&target_path);
        let staging_meta = raisin_hnsw::meta_path(staging_path);

        if !staging_meta.exists() {
            // Unreachable through `receive_index`, which always writes the pair.
            // Guarded anyway because ingesting a lone graph file is the failure
            // this whole module was rewritten to make impossible.
            return Err(Error::storage(format!(
                "Refusing to ingest {}: its `.hnsw.meta` sidecar is missing from staging",
                staging_path.display()
            )));
        }

        info!(
            staging_path = %staging_path.display(),
            target_path = %target_path.display(),
            "Ingesting HNSW index"
        );

        // Create parent directories
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::storage(format!("Failed to create parent directory: {}", e)))?;
        }

        // If a target already exists, back BOTH files up as a pair.
        if target_path.exists() {
            let backup_path = append_suffix(&target_path, ".backup");
            let backup_meta = append_suffix(&target_meta, ".backup");
            warn!(
                target_path = %target_path.display(),
                backup_path = %backup_path.display(),
                "Target file exists, creating backup"
            );

            for stale in [&backup_path, &backup_meta] {
                if stale.exists() {
                    fs::remove_file(stale).await.map_err(|e| {
                        Error::storage(format!("Failed to remove old backup: {}", e))
                    })?;
                }
            }

            fs::rename(&target_path, &backup_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to create backup: {}", e)))?;
            if target_meta.exists() {
                fs::rename(&target_meta, &backup_meta)
                    .await
                    .map_err(|e| Error::storage(format!("Failed to back up sidecar: {}", e)))?;
            }
        }

        // Sidecar first, then the graph: if the process dies between the two,
        // the target has a sidecar and no graph — which reads as "no index",
        // recoverable — rather than a graph with no sidecar, which reads as a
        // bincode index and destroys itself.
        fs::rename(&staging_meta, &target_meta)
            .await
            .map_err(|e| Error::storage(format!("Failed to ingest index sidecar: {}", e)))?;
        fs::rename(staging_path, &target_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to ingest index: {}", e)))?;

        info!("HNSW index ingested successfully");
        Ok(())
    }

    /// Abort index reception and clean up staging files
    pub async fn abort_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<()> {
        let staging_path = self.staging_path(tenant_id, repo_id, branch, partition);
        let staging_meta = raisin_hnsw::meta_path(&staging_path);

        for path in [&staging_path, &staging_meta] {
            if path.exists() {
                fs::remove_file(path).await.map_err(|e| {
                    Error::storage(format!("Failed to clean up staging file: {}", e))
                })?;
            }
        }

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            partition = %partition,
            "Aborted HNSW index receive, staging cleaned up"
        );

        Ok(())
    }
}

/// Append a suffix to a path, never REPLACING an existing extension.
///
/// `Path::with_extension("hnsw.backup")` on `x.hnsw.meta` yields `x.hnsw.backup`
/// — it eats `.meta`. That family of mistake is what gave two branches one index
/// file; it does not get to reappear here.
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

// Implement the raisin_replication trait for HnswIndexReceiver
#[async_trait::async_trait]
impl raisin_replication::HnswIndexReceiver for HnswIndexReceiver {
    async fn receive_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &str,
        data: Vec<u8>,
        expected_crc32: u32,
    ) -> Result<std::path::PathBuf, raisin_replication::CoordinatorError> {
        let partition = parse_partition(partition)?;
        Self::receive_index(
            self,
            tenant_id,
            repo_id,
            branch,
            &partition,
            data,
            expected_crc32,
        )
        .await
        .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }

    async fn ingest_index(
        &self,
        staging_path: &std::path::Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &str,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        let partition = parse_partition(partition)?;
        Self::ingest_index(self, staging_path, tenant_id, repo_id, branch, &partition)
            .await
            .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }

    async fn abort_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &str,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        let partition = parse_partition(partition)?;
        Self::abort_receive(self, tenant_id, repo_id, branch, &partition)
            .await
            .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }
}

/// Validate a partition token that arrived over the wire.
///
/// `raisin-replication` carries it as a `&str` (it cannot depend on
/// `raisin-hnsw`), so this is the trust boundary: a peer must not be able to
/// name `../../` and have it become a path component.
fn parse_partition(token: &str) -> Result<PartitionId, raisin_replication::CoordinatorError> {
    PartitionId::parse(token).ok_or_else(|| {
        raisin_replication::CoordinatorError::Storage(format!(
            "peer sent an invalid HNSW partition token {token:?}"
        ))
    })
}
