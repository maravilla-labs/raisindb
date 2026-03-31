//! HNSW index manager for collecting and loading indexes.

use super::types::HnswIndexMetadata;
use raisin_error::{Error, Result};
use raisin_replication::ReliableFileStreamer;
use std::path::PathBuf;
use tokio::fs;
use tracing::info;

/// HNSW index manager for transferring vector indexes
pub struct HnswIndexManager {
    /// Base directory for HNSW indexes
    /// Typically: data_dir/hnsw/tenant_id/repo_id/branch
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

    /// Get the file path for a specific HNSW index
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    pub fn get_index_path(&self, tenant_id: &str, repo_id: &str, branch: &str) -> PathBuf {
        self.index_base_dir
            .join(tenant_id)
            .join(repo_id)
            .join(format!("{}.hnsw", branch))
    }

    /// Collect HNSW index metadata for a given tenant/repo/branch
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    /// Metadata about the index including size and checksum
    pub async fn collect_index_metadata(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<HnswIndexMetadata>> {
        let index_path = self.get_index_path(tenant_id, repo_id, branch);

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            path = %index_path.display(),
            "Collecting HNSW index metadata"
        );

        // Check if index file exists
        if !index_path.exists() {
            return Ok(None);
        }

        // Get file size
        let metadata = fs::metadata(&index_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to read index metadata: {}", e)))?;

        let size_bytes = metadata.len();

        // Calculate CRC32 checksum
        let crc32 = ReliableFileStreamer::calculate_file_crc32(&index_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to calculate checksum: {}", e)))?;

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            size_bytes = size_bytes,
            crc32 = crc32,
            "HNSW index metadata collected"
        );

        Ok(Some(HnswIndexMetadata {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            size_bytes,
            crc32,
        }))
    }

    /// Load complete HNSW index data for transfer
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    /// Complete file data and CRC32 checksum
    pub async fn load_index_data(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<(Vec<u8>, u32)>> {
        let index_path = self.get_index_path(tenant_id, repo_id, branch);

        if !index_path.exists() {
            return Ok(None);
        }

        info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            branch = %branch,
            path = %index_path.display(),
            "Loading HNSW index data"
        );

        // Read entire file
        let data = fs::read(&index_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to read index file: {}", e)))?;

        // Calculate CRC32
        let crc32 = ReliableFileStreamer::calculate_file_crc32(&index_path)
            .await
            .map_err(|e| Error::storage(format!("Failed to calculate checksum: {}", e)))?;

        info!(size_mb = data.len() / 1_048_576, "HNSW index loaded");

        Ok(Some((data, crc32)))
    }

    /// List all tenant/repo/branch combinations that have HNSW indexes
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

                // Read .hnsw files in repo directory
                let mut file_entries = fs::read_dir(repo_entry.path())
                    .await
                    .map_err(|e| Error::storage(format!("Failed to read repo files: {}", e)))?;

                while let Some(file_entry) = file_entries
                    .next_entry()
                    .await
                    .map_err(|e| Error::storage(format!("Failed to read file entry: {}", e)))?
                {
                    let file_name = file_entry.file_name().to_string_lossy().to_string();

                    // Check if it's a .hnsw file
                    if file_name.ends_with(".hnsw") {
                        // Extract branch name (remove .hnsw extension)
                        if let Some(branch) = file_name.strip_suffix(".hnsw") {
                            indexes.push((tenant_id.clone(), repo_id.clone(), branch.to_string()));
                        }
                    }
                }
            }
        }

        Ok(indexes)
    }
}
