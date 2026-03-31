//! HNSW index receiver for ingesting indexes from other nodes.

use raisin_error::{Error, Result};
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

    /// Receive and verify HNSW index data
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    /// * `data` - Complete index file data
    /// * `expected_crc32` - Expected CRC32 checksum
    ///
    /// # Returns
    /// Path to staged index file
    pub async fn receive_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        data: Vec<u8>,
        expected_crc32: u32,
    ) -> Result<PathBuf> {
        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
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

        // Create staging directory
        let staging_path = self
            .staging_dir
            .join(format!("hnsw_{}_{}_{}.hnsw", tenant_id, repo_id, branch));

        // Write to staging file
        fs::write(&staging_path, &data)
            .await
            .map_err(|e| Error::storage(format!("Failed to write staging file: {}", e)))?;

        info!(
            staging_path = %staging_path.display(),
            "HNSW index received and verified"
        );

        Ok(staging_path)
    }

    /// Ingest index by moving it to the final location
    ///
    /// # Arguments
    /// * `staging_path` - Path to staging file with verified data
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    pub async fn ingest_index(
        &self,
        staging_path: &Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<()> {
        let target_path = self
            .index_base_dir
            .join(tenant_id)
            .join(repo_id)
            .join(format!("{}.hnsw", branch));

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

        // If target already exists, back it up
        if target_path.exists() {
            let backup_path = target_path.with_extension("hnsw.backup");
            warn!(
                target_path = %target_path.display(),
                backup_path = %backup_path.display(),
                "Target file exists, creating backup"
            );

            // Remove old backup if exists
            if backup_path.exists() {
                fs::remove_file(&backup_path)
                    .await
                    .map_err(|e| Error::storage(format!("Failed to remove old backup: {}", e)))?;
            }

            // Move current to backup
            fs::rename(&target_path, &backup_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to create backup: {}", e)))?;
        }

        // Move staging file to target
        fs::rename(staging_path, &target_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to ingest index: {}", e)))?;

        info!("HNSW index ingested successfully");
        Ok(())
    }

    /// Abort index reception and clean up staging file
    pub async fn abort_receive(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        let staging_path = self
            .staging_dir
            .join(format!("hnsw_{}_{}_{}.hnsw", tenant_id, repo_id, branch));

        if staging_path.exists() {
            fs::remove_file(&staging_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to clean up staging file: {}", e)))?;

            info!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                "Aborted HNSW index receive, staging cleaned up"
            );
        }

        Ok(())
    }
}

// Implement the raisin_replication trait for HnswIndexReceiver
#[async_trait::async_trait]
impl raisin_replication::HnswIndexReceiver for HnswIndexReceiver {
    async fn receive_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        data: Vec<u8>,
        expected_crc32: u32,
    ) -> Result<std::path::PathBuf, raisin_replication::CoordinatorError> {
        Self::receive_index(self, tenant_id, repo_id, branch, data, expected_crc32)
            .await
            .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }

    async fn ingest_index(
        &self,
        staging_path: &std::path::Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        Self::ingest_index(self, staging_path, tenant_id, repo_id, branch)
            .await
            .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }

    async fn abort_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<(), raisin_replication::CoordinatorError> {
        Self::abort_receive(self, tenant_id, repo_id, branch)
            .await
            .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }
}
