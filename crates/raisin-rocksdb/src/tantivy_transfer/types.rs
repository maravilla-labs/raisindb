//! Types for Tantivy index transfer.

use raisin_replication::IndexFileInfo;

/// Tantivy index metadata
#[derive(Debug, Clone)]
pub struct TantivyIndexMetadata {
    /// Tenant ID
    pub tenant_id: String,

    /// Repository ID
    pub repo_id: String,

    /// Branch name
    pub branch: String,

    /// List of index files with checksums
    pub files: Vec<IndexFileInfo>,

    /// Total size of all files in bytes
    pub total_size_bytes: u64,
}
