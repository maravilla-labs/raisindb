//! Repository trait implementations for RocksDB

mod archetypes;
mod branches;
mod compound_index;
mod element_types;
mod embedding_storage;
mod excerpt_fetcher;
mod fulltext_job_store;
mod garbage_collection;
mod identity;
mod node_types;
mod nodes;
mod oplog;
mod processing_rules;
mod property_index;
mod reference_index;
mod registry;
mod relations;
mod repository_management;
mod revisions;
mod session;
mod spatial_index;
mod system_updates;
mod tags;
mod tenant_ai_config;
mod tenant_auth_config;
mod tenant_embedding_config;
mod translations;
mod trees;
mod unique_index;
mod versioning;
mod workspaces;

pub use archetypes::ArchetypeRepositoryImpl;
pub use branches::BranchRepositoryImpl;
pub use compound_index::CompoundIndexRepositoryImpl;
pub use element_types::ElementTypeRepositoryImpl;
pub use embedding_storage::{RocksDBEmbeddingJobStore, RocksDBEmbeddingStorage};
pub use excerpt_fetcher::RocksDBExcerptFetcher;
pub use fulltext_job_store::RocksDbJobStore;
pub use garbage_collection::GarbageCollectionRepositoryImpl;
pub use identity::IdentityRepository;
pub use node_types::NodeTypeRepositoryImpl;
pub use nodes::NodeRepositoryImpl;
pub use oplog::{OpLogRepository, OpLogStats};
pub use processing_rules::{ProcessingRulesRepositoryImpl, CF_PROCESSING_RULES};
pub use property_index::PropertyIndexRepositoryImpl;
pub use reference_index::ReferenceIndexRepositoryImpl;
pub use registry::RegistryRepositoryImpl;
pub use relations::RelationRepositoryImpl;
pub use repository_management::RepositoryManagementRepositoryImpl;
pub use revisions::RevisionRepositoryImpl;
pub use session::SessionRepository;
pub use spatial_index::{ProximityResult, SpatialIndexEntry, SpatialIndexRepository};
pub use system_updates::SystemUpdateRepositoryImpl;
pub use tags::TagRepositoryImpl;
pub use tenant_ai_config::TenantAIConfigRepository;
pub use tenant_auth_config::TenantAuthConfigRepository;
pub use tenant_embedding_config::TenantEmbeddingConfigRepository;
pub use translations::RocksDBTranslationRepository;
pub use trees::TreeRepositoryImpl;
pub use unique_index::UniqueIndexManager;
pub use versioning::VersioningRepositoryImpl;
pub use workspaces::WorkspaceRepositoryImpl;

// Re-export helpers for internal use
pub(crate) use nodes::hash_property_value;
pub(crate) use nodes::StorageNode;

// Re-export relation helpers for job handlers
pub(crate) use relations::helpers::{
    deserialize_full_relation, deserialize_relation_ref, get_relation_cf,
    is_tombstone as is_relation_tombstone, TOMBSTONE as RELATION_TOMBSTONE,
};

// Re-export node helpers for job handlers
pub(crate) use nodes::helpers::is_tombstone as is_node_tombstone;
