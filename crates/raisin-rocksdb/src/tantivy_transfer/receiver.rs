//! Tantivy index receiver for ingesting indexes from other nodes.

use raisin_error::{Error, Result};
use raisin_replication::{IndexFileInfo, ReliableFileStreamer};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

/// Tantivy index receiver for ingesting indexes from other nodes
pub struct TantivyIndexReceiver {
    /// Base directory for Tantivy indexes
    index_base_dir: PathBuf,

    /// Staging directory for incoming indexes
    staging_dir: PathBuf,
}

impl TantivyIndexReceiver {
    /// Create a new Tantivy index receiver
    pub fn new(index_base_dir: PathBuf, staging_dir: PathBuf) -> Self {
        Self {
            index_base_dir,
            staging_dir,
        }
    }

    /// Prepare to receive a Tantivy index
    ///
    /// Creates staging directory and returns the path where files should be written
    pub async fn prepare_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<PathBuf> {
        let staging_path = self
            .staging_dir
            .join(format!("tantivy_{}_{}_{}", tenant_id, repo_id, branch));

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            staging_path = %staging_path.display(),
            "Preparing to receive Tantivy index"
        );

        fs::create_dir_all(&staging_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to create staging directory: {}", e)))?;

        Ok(staging_path)
    }

    /// Verify received index
    ///
    /// Validates that all expected files are present with correct checksums
    pub async fn verify_index(
        &self,
        staging_path: &Path,
        expected_files: &[IndexFileInfo],
    ) -> Result<()> {
        info!(
            staging_path = %staging_path.display(),
            num_files = expected_files.len(),
            "Verifying received Tantivy index"
        );

        for file_info in expected_files {
            let file_path = staging_path.join(&file_info.file_name);

            // Check file exists
            if !file_path.exists() {
                return Err(Error::storage(format!(
                    "Missing file in index: {}",
                    file_info.file_name
                )));
            }

            // Verify size
            let metadata = fs::metadata(&file_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to read file metadata: {}", e)))?;

            if metadata.len() != file_info.size_bytes {
                return Err(Error::storage(format!(
                    "File size mismatch for {}: expected {}, got {}",
                    file_info.file_name,
                    file_info.size_bytes,
                    metadata.len()
                )));
            }

            // Verify checksum
            let calculated_crc32 = ReliableFileStreamer::calculate_file_crc32(&file_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to calculate checksum: {}", e)))?;

            if calculated_crc32 != file_info.crc32 {
                return Err(Error::storage(format!(
                    "Checksum mismatch for {}: expected {}, got {}",
                    file_info.file_name, file_info.crc32, calculated_crc32
                )));
            }
        }

        info!("Tantivy index verification successful");
        Ok(())
    }

    /// Ingest index by moving it to the final location
    pub async fn ingest_index(
        &self,
        staging_path: &Path,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<()> {
        let target_dir = self
            .index_base_dir
            .join(tenant_id)
            .join(repo_id)
            .join(branch);

        info!(
            staging_path = %staging_path.display(),
            target_dir = %target_dir.display(),
            "Ingesting Tantivy index"
        );

        // Create parent directories
        if let Some(parent) = target_dir.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::storage(format!("Failed to create parent directory: {}", e)))?;
        }

        // If target already exists, back it up
        if target_dir.exists() {
            let backup_path = target_dir.with_extension("backup");
            warn!(
                target_dir = %target_dir.display(),
                backup_path = %backup_path.display(),
                "Target directory exists, creating backup"
            );

            // Remove old backup if exists
            if backup_path.exists() {
                fs::remove_dir_all(&backup_path)
                    .await
                    .map_err(|e| Error::storage(format!("Failed to remove old backup: {}", e)))?;
            }

            // Move current to backup
            fs::rename(&target_dir, &backup_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to create backup: {}", e)))?;
        }

        // Move staging directory to target
        fs::rename(staging_path, &target_dir)
            .await
            .map_err(|e| Error::storage(format!("Failed to ingest index: {}", e)))?;

        info!("Tantivy index ingested successfully");
        Ok(())
    }

    /// Abort index reception and clean up staging directory
    pub async fn abort_receive(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        let staging_path = self
            .staging_dir
            .join(format!("tantivy_{}_{}_{}", tenant_id, repo_id, branch));

        if staging_path.exists() {
            fs::remove_dir_all(&staging_path).await.map_err(|e| {
                Error::storage(format!("Failed to clean up staging directory: {}", e))
            })?;

            info!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                "Aborted Tantivy index receive, staging cleaned up"
            );
        }

        Ok(())
    }
}

// Implement the raisin_replication trait for TantivyIndexReceiver
#[async_trait::async_trait]
impl raisin_replication::TantivyIndexReceiver for TantivyIndexReceiver {
    async fn prepare_receive(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<std::path::PathBuf, raisin_replication::CoordinatorError> {
        Self::prepare_receive(self, tenant_id, repo_id, branch)
            .await
            .map_err(|e| raisin_replication::CoordinatorError::Storage(e.to_string()))
    }

    async fn verify_index(
        &self,
        staging_path: &std::path::Path,
        expected_files: &[raisin_replication::IndexFileInfo],
    ) -> Result<(), raisin_replication::CoordinatorError> {
        Self::verify_index(self, staging_path, expected_files)
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
