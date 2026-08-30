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
///     spec_hash: Some(87654321),
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

    /// Hash of THIS CHUNK's text.
    ///
    /// Kept with its original meaning — it is a debugging aid and the legacy
    /// staleness signal — but it is NOT the staleness answer: it says nothing
    /// about the embedder, the chunking configuration or the pipeline that
    /// produced the chunk. Use [`Self::spec_hash`] for that.
    pub text_hash: u64,

    /// Hash of EVERY input that decided these vectors — extracted text,
    /// embedder identity, chunking configuration, pipeline version. See
    /// [`crate::spec::EmbeddingSpec`].
    ///
    /// `None` on a row written before the spec hash existed. A row whose spec is
    /// unknown is never treated as current, so it is regenerated once on its
    /// next job and then carries a spec hash like any other. Absent rather than
    /// a sentinel because the value is a hash: no bit pattern is free.
    // No `skip_serializing_if`: the compact (array) MessagePack encoding used
    // by some callers positions fields by index, and an omitted one shifts every
    // field after it.
    #[serde(default)]
    pub spec_hash: Option<u64>,

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
    /// Is this stored row exactly what the current inputs would produce?
    ///
    /// THE staleness question, answered in ONE place so the job handler, an
    /// administrative regenerate and any future sweeper cannot disagree about
    /// what "stale" means.
    ///
    /// It is deliberately conservative in both directions that matter:
    /// - a row with no `spec_hash` (written before spec hashing existed) is
    ///   never current, because we cannot know what produced it;
    /// - `total_chunks` is compared as well as the hash. It is implied by the
    ///   spec, but it is the field an orphan sweep acts on, so a row that
    ///   disagrees about it is not one to trust.
    pub fn is_current_for(&self, spec_hash: u64, total_chunks: usize) -> bool {
        self.spec_hash == Some(spec_hash) && self.total_chunks == total_chunks
    }

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
            spec_hash: Some(12345),
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
        assert_eq!(data.spec_hash, deserialized.spec_hash);
    }

    /// A row written BEFORE spec hashing existed must still deserialize — the
    /// CF is MessagePack and there is no migration — and must read as "spec
    /// unknown", never as "spec matches whatever we happen to ask about".
    ///
    /// The stored format is `to_vec_named` (a map), which is what makes the new
    /// field additive: an absent key falls back to the serde default.
    #[test]
    fn legacy_row_without_spec_hash_is_never_current() {
        #[derive(Serialize)]
        struct LegacyEmbeddingData {
            vector: Vec<f32>,
            embedder_id: EmbedderId,
            embedding_kind: EmbeddingKind,
            source_id: String,
            chunk_index: usize,
            total_chunks: usize,
            chunk_content: Option<String>,
            generated_at: DateTime<Utc>,
            text_hash: u64,
            model: String,
            provider: EmbeddingProvider,
        }

        let legacy = LegacyEmbeddingData {
            vector: vec![0.1, 0.2, 0.3],
            embedder_id: EmbedderId::new("openai", "test-model", 3),
            embedding_kind: EmbeddingKind::Text,
            source_id: "node1".to_string(),
            chunk_index: 0,
            total_chunks: 1,
            chunk_content: Some("test content".to_string()),
            generated_at: Utc::now(),
            text_hash: 12345,
            model: "test-model".to_string(),
            provider: EmbeddingProvider::OpenAI,
        };

        let bytes = rmp_serde::to_vec_named(&legacy).unwrap();
        let read: EmbeddingData = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(read.spec_hash, None, "legacy row must read as spec-unknown");
        assert!(
            !read.is_current_for(12345, 1),
            "an unknown spec must never claim to be current — not even against \
             the legacy text_hash value, which is a DIFFERENT hash of DIFFERENT \
             inputs"
        );
    }

    #[test]
    fn is_current_for_compares_spec_and_chunk_count() {
        #[allow(deprecated)]
        let data = EmbeddingData {
            vector: vec![0.1],
            embedder_id: EmbedderId::new("ollama", "bge-m3", 1),
            embedding_kind: EmbeddingKind::Text,
            source_id: "n".to_string(),
            chunk_index: 0,
            total_chunks: 3,
            chunk_content: None,
            generated_at: Utc::now(),
            text_hash: 1,
            spec_hash: Some(99),
            model: "bge-m3".to_string(),
            provider: EmbeddingProvider::Ollama,
        };

        assert!(data.is_current_for(99, 3));
        assert!(!data.is_current_for(100, 3), "changed spec is stale");
        assert!(
            !data.is_current_for(99, 2),
            "same spec but a different chunk count is not a row to trust"
        );
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
            spec_hash: Some(12345),
            model: "text-embedding-3-small".to_string(),
            provider: EmbeddingProvider::OpenAI,
        };

        let size = data.estimated_size_bytes();

        // 1536 * 4 bytes + ~128 bytes metadata = ~6272 bytes
        assert!(size >= 6000 && size <= 7000, "Size was {}", size);
    }
}
