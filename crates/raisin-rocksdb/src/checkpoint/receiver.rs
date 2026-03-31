//! Checkpoint receiver for ingesting snapshots from other nodes.

use raisin_error::{Error, Result};
use raisin_replication::{ReliableFileStreamer, SstFileInfo};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{error, info, warn};

/// Checkpoint receiver for ingesting snapshots from other nodes
pub struct CheckpointReceiver {
    /// Target data directory (where RocksDB will be replaced)
    data_dir: PathBuf,

    /// Temporary staging directory for incoming checkpoints
    staging_dir: PathBuf,
}

impl CheckpointReceiver {
    /// Create a new checkpoint receiver
    pub fn new(data_dir: PathBuf, staging_dir: PathBuf) -> Self {
        Self {
            data_dir,
            staging_dir,
        }
    }

    /// Prepare to receive a checkpoint
    ///
    /// Creates staging directory and returns the path where files should be written
    pub async fn prepare_receive(&self, snapshot_id: &str) -> Result<PathBuf> {
        let staging_path = self.staging_dir.join(snapshot_id);

        info!(
            snapshot_id = snapshot_id,
            staging_path = %staging_path.display(),
            "Preparing to receive checkpoint"
        );

        fs::create_dir_all(&staging_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to create staging directory: {}", e)))?;

        Ok(staging_path)
    }

    /// Verify received checkpoint
    ///
    /// Validates that all expected files are present with correct checksums
    pub async fn verify_checkpoint(
        &self,
        staging_path: &Path,
        expected_files: &[SstFileInfo],
    ) -> Result<()> {
        info!(
            staging_path = %staging_path.display(),
            num_files = expected_files.len(),
            "Verifying received checkpoint"
        );

        for file_info in expected_files {
            let file_path = staging_path.join(&file_info.file_name);

            if !file_path.exists() {
                return Err(Error::storage(format!(
                    "Missing file in checkpoint: {}",
                    file_info.file_name
                )));
            }

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

        info!("Checkpoint verification successful");
        Ok(())
    }

    /// Ingest checkpoint by replacing the current database
    ///
    /// **IMPORTANT**: This should only be called when the database is not in use!
    pub async fn ingest_checkpoint(&self, staging_path: &Path) -> Result<()> {
        info!(
            staging_path = %staging_path.display(),
            data_dir = %self.data_dir.display(),
            "Ingesting checkpoint (replacing database)"
        );

        // Create backup of current database
        let backup_path = self.data_dir.with_extension("backup");
        if self.data_dir.exists() {
            warn!(
                data_dir = %self.data_dir.display(),
                backup_path = %backup_path.display(),
                "Creating backup of current database before replacement"
            );

            if backup_path.exists() {
                fs::remove_dir_all(&backup_path)
                    .await
                    .map_err(|e| Error::storage(format!("Failed to remove old backup: {}", e)))?;
            }

            fs::rename(&self.data_dir, &backup_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to create backup: {}", e)))?;
        }

        // Move staging directory to data directory
        fs::rename(staging_path, &self.data_dir)
            .await
            .map_err(|e| {
                error!(
                    error = %e,
                    "Failed to replace database with checkpoint, attempting to restore backup"
                );

                // Attempt to restore backup
                if backup_path.exists() {
                    if let Err(restore_err) = std::fs::rename(&backup_path, &self.data_dir) {
                        error!(
                            error = %restore_err,
                            "CRITICAL: Failed to restore backup! Database may be corrupted."
                        );
                    }
                }

                Error::storage(format!("Failed to ingest checkpoint: {}", e))
            })?;

        info!("Checkpoint ingested successfully, database replaced");

        // Clean up backup after successful ingestion
        if backup_path.exists() {
            fs::remove_dir_all(&backup_path).await.ok();
        }

        Ok(())
    }

    /// Abort checkpoint reception and clean up staging directory
    pub async fn abort_receive(&self, snapshot_id: &str) -> Result<()> {
        let staging_path = self.staging_dir.join(snapshot_id);

        if staging_path.exists() {
            fs::remove_dir_all(&staging_path).await.map_err(|e| {
                Error::storage(format!("Failed to clean up staging directory: {}", e))
            })?;

            info!(
                snapshot_id = snapshot_id,
                "Aborted checkpoint receive, staging cleaned up"
            );
        }

        Ok(())
    }
}
