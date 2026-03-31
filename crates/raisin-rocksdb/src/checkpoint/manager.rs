//! Checkpoint manager for creating and managing database snapshots.

use crate::cf;
use raisin_error::{Error, Result};
use raisin_replication::{ReliableFileStreamer, SstFileInfo};
use rocksdb::{checkpoint::Checkpoint, DB};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tracing::{info, warn};

/// RocksDB checkpoint manager for creating and managing database snapshots
pub struct CheckpointManager {
    /// Reference to the RocksDB database
    db: Arc<DB>,

    /// Directory where checkpoints are stored
    checkpoint_dir: PathBuf,
}

/// Checkpoint metadata
#[derive(Debug, Clone)]
pub struct CheckpointMetadata {
    /// Snapshot ID
    pub snapshot_id: String,
    /// Path to checkpoint directory
    pub checkpoint_path: PathBuf,
    /// List of SST files with checksums
    pub sst_files: Vec<SstFileInfo>,
    /// Total size of all files in bytes
    pub total_size_bytes: u64,
    /// Column families included
    pub column_families: Vec<String>,
}

impl CheckpointManager {
    /// Create a new checkpoint manager
    pub fn new(db: Arc<DB>, checkpoint_dir: PathBuf) -> Self {
        Self { db, checkpoint_dir }
    }

    /// Create a new checkpoint (atomic snapshot) of the database
    pub async fn create_checkpoint(&self, snapshot_id: &str) -> Result<CheckpointMetadata> {
        let checkpoint_path = self.checkpoint_dir.join(snapshot_id);

        info!(
            snapshot_id = snapshot_id,
            path = %checkpoint_path.display(),
            "Creating RocksDB checkpoint"
        );

        // Ensure parent directory exists
        if let Some(parent) = checkpoint_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                Error::storage(format!(
                    "Failed to create checkpoint parent directory: {}",
                    e
                ))
            })?;
        }

        // Remove existing checkpoint directory if it exists
        if checkpoint_path.exists() {
            warn!(
                snapshot_id = snapshot_id,
                "Removing existing checkpoint directory"
            );
            fs::remove_dir_all(&checkpoint_path).await.map_err(|e| {
                Error::storage(format!("Failed to remove existing checkpoint: {}", e))
            })?;
        }

        // Create RocksDB checkpoint (atomic snapshot)
        let db = self.db.clone();
        let checkpoint_path_clone = checkpoint_path.clone();

        tokio::task::spawn_blocking(move || {
            let checkpoint = Checkpoint::new(&db)
                .map_err(|e| Error::storage(format!("Failed to create checkpoint: {}", e)))?;
            checkpoint
                .create_checkpoint(&checkpoint_path_clone)
                .map_err(|e| Error::storage(format!("Failed to create checkpoint: {}", e)))
        })
        .await
        .map_err(|e| Error::storage(format!("Checkpoint task failed: {}", e)))??;

        info!(
            snapshot_id = snapshot_id,
            "Checkpoint created, collecting file metadata"
        );

        // Collect all SST files and calculate checksums
        let sst_files = self.collect_sst_files(&checkpoint_path).await?;
        let total_size: u64 = sst_files.iter().map(|f| f.size_bytes).sum();
        let column_families = self.list_column_families();

        info!(
            snapshot_id = snapshot_id,
            num_files = sst_files.len(),
            total_size_mb = total_size / 1_048_576,
            "Checkpoint metadata collected"
        );

        Ok(CheckpointMetadata {
            snapshot_id: snapshot_id.to_string(),
            checkpoint_path,
            sst_files,
            total_size_bytes: total_size,
            column_families,
        })
    }

    /// Collect all files in a checkpoint and calculate their checksums
    async fn collect_sst_files(&self, checkpoint_path: &Path) -> Result<Vec<SstFileInfo>> {
        let mut sst_files = Vec::new();

        let mut entries = fs::read_dir(checkpoint_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to read checkpoint directory: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| Error::storage(format!("Failed to read directory entry: {}", e)))?
        {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            let file_type = entry
                .file_type()
                .await
                .map_err(|e| Error::storage(format!("Failed to read file type: {}", e)))?;

            if file_type.is_file() {
                let metadata = entry
                    .metadata()
                    .await
                    .map_err(|e| Error::storage(format!("Failed to read file metadata: {}", e)))?;

                let size_bytes = metadata.len();

                let crc32 = ReliableFileStreamer::calculate_file_crc32(&path)
                    .await
                    .map_err(|e| Error::storage(format!("Failed to calculate checksum: {}", e)))?;

                sst_files.push(SstFileInfo {
                    file_name,
                    size_bytes,
                    crc32,
                });
            }
        }

        sst_files.sort_by(|a, b| a.file_name.cmp(&b.file_name));

        Ok(sst_files)
    }

    /// List all column families in the database
    fn list_column_families(&self) -> Vec<String> {
        vec![
            cf::NODES.to_string(),
            cf::EMBEDDINGS.to_string(),
            cf::TREES.to_string(),
            cf::REVISIONS.to_string(),
            cf::BRANCHES.to_string(),
            cf::PATH_INDEX.to_string(),
            cf::PROPERTY_INDEX.to_string(),
            cf::REFERENCE_INDEX.to_string(),
            cf::RELATION_INDEX.to_string(),
            cf::ORDER_INDEX.to_string(),
            cf::ORDERED_CHILDREN.to_string(),
            cf::NODE_TYPES.to_string(),
            cf::ARCHETYPES.to_string(),
            cf::ELEMENT_TYPES.to_string(),
            cf::WORKSPACES.to_string(),
            cf::TAGS.to_string(),
            cf::REGISTRY.to_string(),
            cf::TRANSLATION_DATA.to_string(),
            cf::BLOCK_TRANSLATIONS.to_string(),
            cf::TRANSLATION_INDEX.to_string(),
            cf::JOB_DATA.to_string(),
            cf::JOB_METADATA.to_string(),
            cf::TENANT_EMBEDDING_CONFIG.to_string(),
            cf::FULLTEXT_JOBS.to_string(),
            cf::EMBEDDING_JOBS.to_string(),
            cf::OPERATION_LOG.to_string(),
        ]
    }

    /// Clean up old checkpoints
    pub async fn cleanup_old_checkpoints(&self, keep_latest: usize) -> Result<usize> {
        let mut checkpoints = Vec::new();

        let mut entries = fs::read_dir(&self.checkpoint_dir)
            .await
            .map_err(|e| Error::storage(format!("Failed to read checkpoint directory: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| Error::storage(format!("Failed to read directory entry: {}", e)))?
        {
            if entry
                .file_type()
                .await
                .ok()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                let metadata = entry.metadata().await.ok();
                if let Some(meta) = metadata {
                    if let Ok(modified) = meta.modified() {
                        checkpoints.push((entry.path(), modified));
                    }
                }
            }
        }

        // Sort by modification time (newest first)
        checkpoints.sort_by(|a, b| b.1.cmp(&a.1));

        let mut removed = 0;
        for (path, _) in checkpoints.iter().skip(keep_latest) {
            match fs::remove_dir_all(path).await {
                Ok(_) => {
                    info!(path = %path.display(), "Removed old checkpoint");
                    removed += 1;
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to remove checkpoint");
                }
            }
        }

        Ok(removed)
    }

    /// Delete a specific checkpoint
    pub async fn delete_checkpoint(&self, snapshot_id: &str) -> Result<()> {
        let checkpoint_path = self.checkpoint_dir.join(snapshot_id);

        if checkpoint_path.exists() {
            fs::remove_dir_all(&checkpoint_path)
                .await
                .map_err(|e| Error::storage(format!("Failed to delete checkpoint: {}", e)))?;

            info!(snapshot_id = snapshot_id, "Checkpoint deleted");
        }

        Ok(())
    }
}
