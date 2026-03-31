//! Legacy data types used during format migrations
//!
//! These structs represent old serialization formats that existed before
//! schema changes. They are used to deserialize old data and convert it
//! to the current format.

use serde::{Deserialize, Serialize};

/// Old RelationRef format (before schema change to add target_node_type and relation_type)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct OldRelationRef {
    pub target: String,
    pub workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// Old FullRelation format (before schema change to add node types and relation_type)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct OldFullRelation {
    pub source_id: String,
    pub source_workspace: String,
    pub target_id: String,
    pub target_workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// Old TenantEmbeddingConfig format (before removing node_type_settings)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OldTenantEmbeddingConfig {
    pub tenant_id: String,
    pub enabled: bool,
    pub provider: raisin_embeddings::config::EmbeddingProvider,
    pub model: String,
    pub dimensions: usize,
    #[serde(default)]
    pub api_key_encrypted: Option<Vec<u8>>,
    pub include_name: bool,
    pub include_path: bool,
    pub max_embeddings_per_repo: Option<usize>,
    #[serde(default)]
    pub node_type_settings: std::collections::HashMap<String, serde_json::Value>,
}

/// Old PersistedJobEntry format (before adding next_retry_at field)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct OldPersistedJobEntry {
    pub id: String,
    pub job_type: raisin_storage::jobs::JobType,
    pub status: raisin_storage::jobs::JobStatus,
    pub tenant: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub progress: Option<f32>,
    pub result: Option<serde_json::Value>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
    pub timeout_seconds: u64,
}

/// Old JobType format (before string serialization)
///
/// This uses serde's default enum serialization (as JSON objects).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename = "JobType")]
pub(super) enum OldJobType {
    IntegrityScan,
    IndexRebuild,
    IndexVerify,
    Compaction,
    Backup,
    Restore,
    OrphanCleanup,
    Repair,
    FulltextVerify,
    FulltextRebuild,
    FulltextOptimize,
    FulltextPurge,
    VectorVerify,
    VectorRebuild,
    VectorOptimize,
    VectorRestore,
    FulltextIndex {
        node_id: String,
        operation: raisin_storage::jobs::IndexOperation,
    },
    FulltextBranchCopy {
        source_branch: String,
    },
    EmbeddingGenerate {
        node_id: String,
    },
    EmbeddingDelete {
        node_id: String,
    },
    EmbeddingBranchCopy {
        source_branch: String,
    },
    Custom(String),
}

/// Old PersistedJobEntry format (with old JobType serialization)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct OldPersistedJobEntryV3 {
    pub id: String,
    pub job_type: OldJobType,
    pub status: raisin_storage::jobs::JobStatus,
    pub tenant: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub progress: Option<f32>,
    pub result: Option<serde_json::Value>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
    pub timeout_seconds: u64,
    pub next_retry_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Old RevisionMeta format (before adding operation field)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct OldRevisionMeta {
    pub revision: u64,
    pub parent: Option<u64>,
    pub branch: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub actor: String,
    pub message: String,
    pub is_system: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_nodes: Vec<raisin_storage::NodeChangeInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_node_types: Vec<raisin_storage::NodeTypeChangeInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_archetypes: Vec<raisin_storage::ArchetypeChangeInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_element_types: Vec<raisin_storage::ElementTypeChangeInfo>,
}

/// Convert old JobType to new JobType (via string)
pub(super) fn convert_old_job_type(old: OldJobType) -> raisin_storage::jobs::JobType {
    use raisin_storage::jobs::JobType;
    match old {
        OldJobType::IntegrityScan => JobType::IntegrityScan,
        OldJobType::IndexRebuild => JobType::IndexRebuild,
        OldJobType::IndexVerify => JobType::IndexVerify,
        OldJobType::Compaction => JobType::Compaction,
        OldJobType::Backup => JobType::Backup,
        OldJobType::Restore => JobType::Restore,
        OldJobType::OrphanCleanup => JobType::OrphanCleanup,
        OldJobType::Repair => JobType::Repair,
        OldJobType::FulltextVerify => JobType::FulltextVerify,
        OldJobType::FulltextRebuild => JobType::FulltextRebuild,
        OldJobType::FulltextOptimize => JobType::FulltextOptimize,
        OldJobType::FulltextPurge => JobType::FulltextPurge,
        OldJobType::VectorVerify => JobType::VectorVerify,
        OldJobType::VectorRebuild => JobType::VectorRebuild,
        OldJobType::VectorOptimize => JobType::VectorOptimize,
        OldJobType::VectorRestore => JobType::VectorRestore,
        OldJobType::FulltextIndex { node_id, operation } => {
            JobType::FulltextIndex { node_id, operation }
        }
        OldJobType::FulltextBranchCopy { source_branch } => {
            JobType::FulltextBranchCopy { source_branch }
        }
        OldJobType::EmbeddingGenerate { node_id } => JobType::EmbeddingGenerate { node_id },
        OldJobType::EmbeddingDelete { node_id } => JobType::EmbeddingDelete { node_id },
        OldJobType::EmbeddingBranchCopy { source_branch } => {
            JobType::EmbeddingBranchCopy { source_branch }
        }
        OldJobType::Custom(name) => JobType::Custom(name),
    }
}
