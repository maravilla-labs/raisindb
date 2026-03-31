//! Tantivy index manager for collecting metadata and listing indexes.

use super::types::TantivyIndexMetadata;
use raisin_error::{Error, Result};
use raisin_replication::{IndexFileInfo, ReliableFileStreamer};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::info;

/// Tantivy index manager for transferring fulltext indexes
pub struct TantivyIndexManager {
    /// Base directory for Tantivy indexes
    /// Typically: data_dir/tantivy/tenant_id/repo_id/branch
    index_base_dir: PathBuf,
}

impl TantivyIndexManager {
    /// Create a new Tantivy index manager
    pub fn new(index_base_dir: PathBuf) -> Self {
        Self { index_base_dir }
    }

    /// Get the directory path for a specific index
    pub fn get_index_dir(&self, tenant_id: &str, repo_id: &str, branch: &str) -> PathBuf {
        self.index_base_dir
            .join(tenant_id)
            .join(repo_id)
            .join(branch)
    }

    /// Collect all Tantivy index files for a given tenant/repo/branch
    pub async fn collect_index_metadata(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<TantivyIndexMetadata> {
        let index_dir = self.get_index_dir(tenant_id, repo_id, branch);

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            path = %index_dir.display(),
            "Collecting Tantivy index metadata"
        );

        // Check if index directory exists
        if !index_dir.exists() {
            return Ok(TantivyIndexMetadata {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                branch: branch.to_string(),
                files: Vec::new(),
                total_size_bytes: 0,
            });
        }

        // Collect all files in the index directory
        let files = self.collect_index_files(&index_dir).await?;
        let total_size: u64 = files.iter().map(|f| f.size_bytes).sum();

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            num_files = files.len(),
            total_size_mb = total_size / 1_048_576,
            "Tantivy index metadata collected"
        );

        Ok(TantivyIndexMetadata {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            files,
            total_size_bytes: total_size,
        })
    }

    /// Collect all files in a Tantivy index directory
    async fn collect_index_files(&self, index_dir: &Path) -> Result<Vec<IndexFileInfo>> {
        let mut index_files = Vec::new();

        let mut entries = fs::read_dir(index_dir)
            .await
            .map_err(|e| Error::storage(format!("Failed to read index directory: {}", e)))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| Error::storage(format!("Failed to read directory entry: {}", e)))?
        {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Skip subdirectories (Tantivy indexes are flat)
            if !path.is_file() {
                continue;
            }

            let metadata = entry
                .metadata()
                .await
                .map_err(|e| Error::storage(format!("Failed to read file metadata: {}", e)))?;

            let size_bytes = metadata.len();

            // Calculate CRC32 checksum
            let crc32 = ReliableFileStreamer::calculate_file_crc32(&path)
                .await
                .map_err(|e| Error::storage(format!("Failed to calculate checksum: {}", e)))?;

            index_files.push(IndexFileInfo {
                file_name,
                size_bytes,
                crc32,
            });
        }

        // Sort by filename for deterministic ordering
        index_files.sort_by(|a, b| a.file_name.cmp(&b.file_name));

        Ok(index_files)
    }

    /// List all tenant/repo/branch combinations that have Tantivy indexes
    pub async fn list_all_indexes(&self) -> Result<Vec<(String, String, String)>> {
        let mut indexes = Vec::new();

        if !self.index_base_dir.exists() {
            return Ok(indexes);
        }

        // Read tenant directories
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

            // Read repo directories
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

                // Read branch directories
                let mut branch_entries = fs::read_dir(repo_entry.path()).await.map_err(|e| {
                    Error::storage(format!("Failed to read branch directory: {}", e))
                })?;

                while let Some(branch_entry) = branch_entries
                    .next_entry()
                    .await
                    .map_err(|e| Error::storage(format!("Failed to read branch entry: {}", e)))?
                {
                    if !branch_entry.path().is_dir() {
                        continue;
                    }

                    let branch = branch_entry.file_name().to_string_lossy().to_string();
                    indexes.push((tenant_id.clone(), repo_id.clone(), branch));
                }
            }
        }

        Ok(indexes)
    }
}
