//! Data models for vector embeddings and embedding jobs.

use chrono::{DateTime, Utc};
use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};

use crate::config::EmbeddingProvider;

/// Stored embedding data for a node at a specific revision.
///
/// This structure is stored in both:
/// - RocksDB `embeddings` CF for direct access and revision history
/// - HNSW index files for fast KNN search
///
/// # Storage Format (Multi-Model)
///
/// **RocksDB Key:** `{tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash:11}\0{kind:1}\0{source_id}\0{chunk_idx:04}\0{revision:HLC:16bytes}`
/// **RocksDB Value:** MessagePack-encoded `EmbeddingData`
///
/// The embedder_hash is a stable 11-character hash identifying the embedding model,
/// allowing multiple embedding models to coexist in the same database.
///
/// # Example
///
/// ```rust,ignore
/// use raisin_embeddings::models::EmbeddingData;
/// use raisin_ai::config::{EmbedderId, EmbeddingKind};
///
/// let embedder = EmbedderId::new("openai", "text-embedding-3-small", 1536);
/// let embedding = EmbeddingData {
///     vector: vec![0.1, 0.2, 0.3],  // Simplified 3D vector
///     embedder_id: embedder,
///     embedding_kind: EmbeddingKind::Text,
///     source_id: "node123".to_string(),
///     chunk_index: 0,
///     total_chunks: 1,
///     chunk_content: Some("Sample text".to_string()),
///     generated_at: chrono::Utc::now(),
///     text_hash: 12345678,
///
///     // Legacy fields (deprecated but kept for backward compatibility)
///     model: "text-embedding-3-small".to_string(),
///     provider: EmbeddingProvider::OpenAI,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    /// The actual embedding vector (typically 1536 or 3072 dimensions)
    pub vector: Vec<f32>,

    /// Embedder identity (provider + model + dimensions + tokenizer)
    /// This uniquely identifies the embedding configuration
    pub embedder_id: EmbedderId,

    /// Type of embedding content (text or image)
    pub embedding_kind: EmbeddingKind,

    /// Source identifier (node ID for text, asset ID for images)
    /// Renamed from node_id for clarity
    pub source_id: String,

    /// Chunk index for multi-chunk text embeddings (0-based)
    /// For single-chunk or image embeddings, this is 0
    pub chunk_index: usize,

    /// Total number of chunks for this source
    /// For images, this is always 1
    pub total_chunks: usize,

    /// Optional text excerpt from this chunk for display/debugging
    /// None for image embeddings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_content: Option<String>,

    /// When generated
    pub generated_at: DateTime<Utc>,

    /// Hash of source text (to detect if re-embedding needed)
    /// This allows us to skip re-generation if node content hasn't changed
    pub text_hash: u64,

    // =========================================================================
    // LEGACY FIELDS (Deprecated - kept for backward compatibility)
    // =========================================================================
    /// Model used to generate (e.g., "text-embedding-3-small")
    /// DEPRECATED: Use embedder_id.model instead
    #[deprecated(note = "Use embedder_id.model instead")]
    #[serde(default = "default_model")]
    pub model: String,

    /// Provider (OpenAI, Claude, Ollama)
    /// DEPRECATED: Use embedder_id.provider instead
    #[deprecated(note = "Use embedder_id.provider instead")]
    #[serde(default = "default_provider")]
    pub provider: EmbeddingProvider,
}

// Default values for legacy fields during deserialization
fn default_model() -> String {
    "unknown".to_string()
}

fn default_provider() -> EmbeddingProvider {
    EmbeddingProvider::OpenAI
}

impl EmbeddingData {
    /// Estimate memory usage in bytes
    pub fn estimated_size_bytes(&self) -> usize {
        // Vector: f32 = 4 bytes per element
        let vector_size = self.vector.len() * 4;

        // Other fields (approximate)
        let metadata_size = 128; // model string, provider enum, datetime, hash

        vector_size + metadata_size
    }
}

/// Background job for embedding generation.
///
/// Jobs are enqueued in response to node lifecycle events:
/// - `AddNode`: Generate embedding for a newly created or updated node
/// - `DeleteNode`: Remove embedding from HNSW index
/// - `BranchCreated`: Copy HNSW index directory for the new branch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingJob {
    /// Unique job identifier
    pub job_id: String,

    /// Type of embedding operation
    pub kind: EmbeddingJobKind,

    /// Tenant identifier
    pub tenant_id: String,

    /// Repository identifier
    pub repo_id: String,

    /// Branch name
    pub branch: String,

    /// Workspace identifier
    pub workspace_id: String,

    /// Revision (full HLC for precise node retrieval)
    pub revision: HLC,

    /// Node ID (for AddNode and DeleteNode)
    pub node_id: Option<String>,

    /// Source branch (for BranchCreated)
    pub source_branch: Option<String>,

    /// When the job was created
    pub created_at: DateTime<Utc>,
}

/// Type of embedding job operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum EmbeddingJobKind {
    /// Generate and store embedding for a node
    AddNode,

    /// Remove embedding from HNSW index
    DeleteNode,

    /// Copy HNSW index directory when a branch is created
    BranchCreated,
}

impl EmbeddingJob {
    /// Create a new AddNode job
    pub fn add_node(
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        node_id: String,
        revision: HLC,
    ) -> Self {
        Self {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: EmbeddingJobKind::AddNode,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            revision,
            node_id: Some(node_id),
            source_branch: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new DeleteNode job
    pub fn delete_node(
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        node_id: String,
        revision: HLC,
    ) -> Self {
        Self {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: EmbeddingJobKind::DeleteNode,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            revision,
            node_id: Some(node_id),
            source_branch: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new BranchCreated job
    pub fn branch_created(
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        source_branch: String,
        revision: HLC,
    ) -> Self {
        Self {
            job_id: uuid::Uuid::new_v4().to_string(),
            kind: EmbeddingJobKind::BranchCreated,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            revision,
            node_id: None,
            source_branch: Some(source_branch),
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedding_data_serialization() {
        let embedder_id = raisin_ai::config::EmbedderId::new("openai", "test-model", 3);

        #[allow(deprecated)]
        let data = EmbeddingData {
            vector: vec![0.1, 0.2, 0.3],
            embedder_id: embedder_id.clone(),
            embedding_kind: raisin_ai::config::EmbeddingKind::Text,
            source_id: "node1".to_string(),
            chunk_index: 0,
            total_chunks: 1,
            chunk_content: Some("test content".to_string()),
            generated_at: Utc::now(),
            text_hash: 12345,
            model: "test-model".to_string(),
            provider: EmbeddingProvider::OpenAI,
        };

        // Test MessagePack serialization
        let bytes = rmp_serde::to_vec(&data).unwrap();
        let deserialized: EmbeddingData = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(data.vector, deserialized.vector);
        assert_eq!(data.source_id, deserialized.source_id);
        assert_eq!(data.text_hash, deserialized.text_hash);
        assert_eq!(data.chunk_index, deserialized.chunk_index);
        assert_eq!(data.total_chunks, deserialized.total_chunks);
    }

    #[test]
    fn test_embedding_job_creation() {
        let revision = HLC::new(42, 0);
        let job = EmbeddingJob::add_node(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            "ws1".to_string(),
            "node1".to_string(),
            revision,
        );

        assert_eq!(job.kind, EmbeddingJobKind::AddNode);
        assert_eq!(job.tenant_id, "tenant1");
        assert_eq!(job.node_id, Some("node1".to_string()));
        assert_eq!(job.revision.timestamp_ms, 42);
        assert!(job.source_branch.is_none());
    }

    #[test]
    fn test_branch_created_job() {
        let revision = HLC::new(10, 0);
        let job = EmbeddingJob::branch_created(
            "tenant1".to_string(),
            "repo1".to_string(),
            "feature".to_string(),
            "ws1".to_string(),
            "main".to_string(),
            revision,
        );

        assert_eq!(job.kind, EmbeddingJobKind::BranchCreated);
        assert_eq!(job.source_branch, Some("main".to_string()));
        assert!(job.node_id.is_none());
    }

    #[test]
    fn test_embedding_data_size_estimate() {
        let embedder_id =
            raisin_ai::config::EmbedderId::new("openai", "text-embedding-3-small", 1536);

        #[allow(deprecated)]
        let data = EmbeddingData {
            vector: vec![0.0; 1536], // Typical OpenAI embedding size
            embedder_id,
            embedding_kind: raisin_ai::config::EmbeddingKind::Text,
            source_id: "node1".to_string(),
            chunk_index: 0,
            total_chunks: 1,
            chunk_content: Some("Sample text".to_string()),
            generated_at: Utc::now(),
            text_hash: 12345,
            model: "text-embedding-3-small".to_string(),
            provider: EmbeddingProvider::OpenAI,
        };

        let size = data.estimated_size_bytes();

        // 1536 * 4 bytes + ~128 bytes metadata = ~6272 bytes
        assert!(size >= 6000 && size <= 7000, "Size was {}", size);
    }
}
