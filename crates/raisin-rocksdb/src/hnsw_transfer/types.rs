//! Types for HNSW index transfer.

/// HNSW index metadata
#[derive(Debug, Clone)]
pub struct HnswIndexMetadata {
    /// Tenant ID
    pub tenant_id: String,

    /// Repository ID
    pub repo_id: String,

    /// Branch name
    pub branch: String,

    /// Which embedding space, `{embedder_hash}{kind}`
    pub partition: raisin_hnsw::PartitionId,

    /// Bundle size in bytes (graph + sidecar)
    pub size_bytes: u64,

    /// CRC32 checksum
    pub crc32: u32,
}
