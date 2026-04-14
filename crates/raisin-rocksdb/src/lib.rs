// TODO(v0.2): Update deprecated API usages to new methods and clean up unused code
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_assignments)]

//! RocksDB storage backend implementation for RaisinDB.
//!
//! This crate implements the storage traits defined in `raisin-storage` using RocksDB
//! as the underlying storage engine.
//!
//! # Architecture
//!
//! - Uses separate column families for different data types
//! - Implements revision-aware indexing with descending revision encoding
//! - Supports multi-tenant, multi-repository operation
//! - Provides Git-like branching and tagging semantics
//!
//! # Column Families
//!
//! - `nodes` - Node blobs keyed by {tenant}/{repo}/{branch}/{workspace}/{id}
//! - `path_index` - Path-to-node mappings for hierarchical navigation
//! - `property_index` - Property value indexes for fast queries
//! - `reference_index` - Reference indexes (forward and reverse)
//! - `order_index` - Sibling ordering indexes
//! - `node_types` - NodeType schemas
//! - `workspaces` - Workspace metadata
//! - `branches` - Branch metadata and HEAD pointers
//! - `tags` - Tag-to-revision mappings
//! - `revisions` - Revision metadata and commit history
//! - `trees` - Content-addressed tree storage
//! - `registry` - Tenant and deployment registration
//! - `workspace_deltas` - Workspace delta operations (draft storage)
//! - `translation_data` - Per-locale translation overlays for nodes
//! - `block_translations` - Block-level translations indexed by UUID
//! - `translation_index` - Reverse index for translation queries

use raisin_error::Result;
use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options, DB};
use std::path::Path;

mod admin_user_store;
mod api_key_store;
mod auth_service;
pub mod checkpoint;
pub mod config;
mod constants;
mod error_ext;
pub mod fractional_index;
pub mod graph;
pub mod hnsw_transfer;
mod jobs;
mod keys;
pub mod lazy_indexing;
pub mod management;
pub mod monitoring;
mod prefix_transform;
pub mod replication;
pub mod repositories;
pub mod security;
pub mod spatial;
mod storage;
pub mod tantivy_transfer;
mod tombstones;
mod transaction;

pub use admin_user_store::AdminUserStore;
pub use api_key_store::ApiKeyStore;
pub use auth_service::{AdminClaims, AuthService};
pub use checkpoint::{CheckpointManager, CheckpointMetadata, CheckpointReceiver};
pub use config::{CompressionType, ReplicationPeerConfig, RocksDBConfig, TenantLimits};
pub use hnsw_transfer::{HnswIndexManager, HnswIndexMetadata, HnswIndexReceiver};
pub use jobs::{
    create_trigger_matcher,
    // Job dispatcher types
    dispatcher::DispatcherStats,
    dispatcher::JobDispatcher,
    // Flow job scheduler
    flow_scheduler::get_flow_job_scheduler,
    // Package installation callback types
    BinaryRetrievalCallback,
    BinaryStorageCallback,
    BinaryUploadCallback,
    // Trigger registry exports
    CachedTrigger,
    CopyTreeExecutorCallback,
    // Dry run types for package install preview
    DryRunActionCounts,
    DryRunLogEntry,
    DryRunResult,
    DryRunSummary,
    FunctionEnabledChecker,
    FunctionExecutionResult,
    // Function execution callback types
    FunctionExecutorCallback,
    JobDataStore,
    // Job metadata store types
    JobMetadataStore,
    // AIToolCall execution callback type
    NodeCreatorCallback,
    // Package install types
    PackageInstallHandler,
    PackageInstallMode,
    PersistedJobEntry,
    // Upload handlers
    ResumableUploadHandler,
    // Flow runtime callbacks
    RocksDBFlowCallbacks,
    ScheduledTriggerFinderCallback,
    ScheduledTriggerMatch,
    SqlExecutorCallback,
    TriggerFilters,
    TriggerMatch,
    TriggerMatcherCallback,
    TriggerRegistry,
    UnifiedJobEventHandler,
    UploadSessionCleanupHandler,
};
pub use lazy_indexing::{BuildResult, LazyIndexManager};
pub use management::{DimensionMismatch, HnswManagement, VectorRebuildStats, VerificationReport};
pub use replication::OperationCapture;
pub use repositories::{
    OpLogRepository, OpLogStats, ProximityResult, RocksDBEmbeddingJobStore,
    RocksDBEmbeddingStorage, RocksDBTranslationRepository, RocksDbJobStore, SpatialIndexEntry,
    SpatialIndexRepository, SystemUpdateRepositoryImpl, TenantAIConfigRepository,
    TenantEmbeddingConfigRepository,
};

// Re-export StorageNode for internal use across modules
pub(crate) use repositories::StorageNode;
pub use tantivy_transfer::{TantivyIndexManager, TantivyIndexMetadata, TantivyIndexReceiver};

// Re-export replication handlers for external use
pub use jobs::handlers::replication_sync::ReplicationSyncHandler;
pub use storage::{RestoreStats, RocksDBStorage};
pub use transaction::RocksDBTransaction;

/// Column family names used by RocksDB
pub mod cf {
    pub const NODES: &str = "nodes";
    pub const PATH_INDEX: &str = "path_index";
    pub const PROPERTY_INDEX: &str = "property_index";
    pub const REFERENCE_INDEX: &str = "reference_index";
    pub const RELATION_INDEX: &str = "relation_index"; // Graph relations index
    pub const ORDER_INDEX: &str = "order_index";
    pub const ORDERED_CHILDREN: &str = "ordered_children"; // Revision-aware child ordering
    pub const NODE_TYPES: &str = "node_types";
    pub const ARCHETYPES: &str = "archetypes";
    pub const ELEMENT_TYPES: &str = "element_types";
    pub const WORKSPACES: &str = "workspaces";
    pub const BRANCHES: &str = "branches";
    pub const TAGS: &str = "tags";
    pub const REVISIONS: &str = "revisions";
    pub const TREES: &str = "trees";
    pub const REGISTRY: &str = "registry";
    pub const WORKSPACE_DELTAS: &str = "workspace_deltas";
    pub const VERSIONS: &str = "versions";
    pub const FULLTEXT_JOBS: &str = "fulltext_jobs";
    pub const TENANT_EMBEDDING_CONFIG: &str = "tenant_embedding_config";
    pub const EMBEDDINGS: &str = "embeddings";
    pub const EMBEDDING_JOBS: &str = "embedding_jobs";
    pub const JOB_DATA: &str = "job_data"; // Stores JobContext by job_id
    pub const JOB_METADATA: &str = "job_metadata"; // Stores JobEntry metadata for persistence
    pub const QUERY_EMBEDDINGS: &str = "query_embeddings"; // Cache for EMBEDDING() function results
    pub const TENANT_AI_CONFIG: &str = "tenant_ai_config"; // Unified AI/LLM provider configuration per tenant
    pub const TENANT_AUTH_CONFIG: &str = "tenant_auth_config"; // Authentication configuration per tenant

    // Translation system column families
    pub const TRANSLATION_DATA: &str = "translation_data"; // LocaleOverlay data per node/locale/revision
    pub const BLOCK_TRANSLATIONS: &str = "block_translations"; // Block-level translations by UUID
    pub const TRANSLATION_INDEX: &str = "translation_index"; // Reverse index: locale -> nodes
    pub const TRANSLATION_HASHES: &str = "translation_hashes"; // Hash records for staleness detection

    // Admin user management
    pub const ADMIN_USERS: &str = "admin_users"; // Database admin users for authentication

    // Replication system
    pub const OPERATION_LOG: &str = "operation_log"; // CRDT operation log for clustering
    pub const APPLIED_OPS: &str = "applied_ops"; // Applied operation IDs for idempotency (per-node state)

    // Lazy indexing system (local node tracking, not replicated)
    pub const INDEX_STATUS: &str = "index_status"; // Tracks last indexed revision per tenant/repo/branch

    // Reverse path lookup (node_id → path) for StorageNode optimization
    pub const NODE_PATH: &str = "node_path"; // Maps node_id to its path for O(1) move operations

    // Geospatial indexing (geohash-based for PostGIS-compatible ST_* queries)
    pub const SPATIAL_INDEX: &str = "spatial_index"; // Geohash-based spatial index for geometry properties

    // Compound indexes for multi-column queries
    pub const COMPOUND_INDEX: &str = "compound_index"; // Multi-column compound indexes for ORDER BY + filter queries

    // Unique property constraint index
    pub const UNIQUE_INDEX: &str = "unique_index"; // Enforces unique property constraints per workspace

    // System updates tracking
    pub const SYSTEM_UPDATE_HASHES: &str = "system_update_hashes"; // Tracks applied NodeType/Workspace hashes per repository

    // Identity and authentication system
    pub const IDENTITIES: &str = "identities"; // Global identities per tenant (auth system)
    pub const IDENTITY_EMAIL_INDEX: &str = "identity_email_index"; // Email -> identity_id lookup index
    pub const SESSIONS: &str = "sessions"; // Active sessions for identities

    // Graph algorithm precomputation cache
    // Key format (branch mode): <repo_id>:branch:<branch_id>:<config_id>:<node_id>
    // Key format (revision mode): <repo_id>:rev:<revision_id>:<config_id>:<node_id>
    // Stores precomputed graph algorithm results (PageRank, Louvain, etc.)
    pub const GRAPH_CACHE: &str = "graph_cache";

    // Graph projection configuration per branch
    // Key format: {tenant}\0{repo}\0graph_projection\0{branch}\0{config_id}
    // Stores graph projection configurations for subgraph extraction
    pub const GRAPH_PROJECTION: &str = "graph_projection";

    // AI processing rules per repository
    // Key format: {tenant_id}\0{repo_id}
    // Stores ProcessingRuleSet for content processing configuration
    pub const PROCESSING_RULES: &str = "processing_rules";
}

/// Get all column family names
pub(crate) fn all_column_families() -> Vec<&'static str> {
    vec![
        cf::NODES,
        cf::PATH_INDEX,
        cf::PROPERTY_INDEX,
        cf::REFERENCE_INDEX,
        cf::RELATION_INDEX,
        cf::ORDER_INDEX,
        cf::ORDERED_CHILDREN,
        cf::NODE_TYPES,
        cf::ARCHETYPES,
        cf::ELEMENT_TYPES,
        cf::WORKSPACES,
        cf::BRANCHES,
        cf::TAGS,
        cf::REVISIONS,
        cf::TREES,
        cf::REGISTRY,
        cf::WORKSPACE_DELTAS,
        cf::VERSIONS,
        cf::FULLTEXT_JOBS,
        cf::TENANT_EMBEDDING_CONFIG,
        cf::EMBEDDINGS,
        cf::EMBEDDING_JOBS,
        cf::JOB_DATA,
        cf::JOB_METADATA,
        cf::QUERY_EMBEDDINGS,
        cf::TENANT_AI_CONFIG,
        cf::TENANT_AUTH_CONFIG,
        cf::TRANSLATION_DATA,
        cf::BLOCK_TRANSLATIONS,
        cf::TRANSLATION_INDEX,
        cf::TRANSLATION_HASHES,
        cf::ADMIN_USERS,
        cf::OPERATION_LOG,
        cf::APPLIED_OPS,
        cf::INDEX_STATUS,
        cf::NODE_PATH,
        cf::SPATIAL_INDEX,
        cf::COMPOUND_INDEX,
        cf::UNIQUE_INDEX,
        cf::SYSTEM_UPDATE_HASHES,
        cf::IDENTITIES,
        cf::IDENTITY_EMAIL_INDEX,
        cf::SESSIONS,
        cf::GRAPH_CACHE,
        cf::GRAPH_PROJECTION,
        cf::PROCESSING_RULES,
    ]
}

/// Create column family descriptors with optimized options
pub(crate) fn create_column_family_descriptors() -> Vec<ColumnFamilyDescriptor> {
    let mut cfs = Vec::new();
    let mut default_opts = Options::default();
    default_opts.set_compression_type(rocksdb::DBCompressionType::Lz4);

    for cf_name in all_column_families() {
        let mut opts = default_opts.clone();

        // Special configuration for ORDERED_CHILDREN CF
        // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0...
        // Prefix up to parent_id for efficient queries
        if cf_name == cf::ORDERED_CHILDREN {
            // Use custom delimiter-based prefix extractor
            // Extracts prefix up to the 6th null byte (after parent_id)
            // This enables prefix bloom filters for efficient scans of children under a parent
            opts.set_prefix_extractor(prefix_transform::create_ordered_children_prefix());
        }

        // Enable bloom filters for PROPERTY_INDEX CF
        // This improves performance for negative lookups (property doesn't exist)
        // by avoiding disk I/O for non-existent keys
        if cf_name == cf::PROPERTY_INDEX {
            let mut block_opts = rocksdb::BlockBasedOptions::default();
            // 10 bits per key gives ~1% false positive rate
            block_opts.set_bloom_filter(10.0, false);
            // Use ribbon filter for better space efficiency (requires RocksDB 6.15+)
            // block_opts.set_ribbon_filter(10.0);
            opts.set_block_based_table_factory(&block_opts);
        }

        // Special configuration for SPATIAL_INDEX CF (geohash-based)
        // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}
        // Optimized for geohash prefix scans (proximity queries via ring expansion)
        if cf_name == cf::SPATIAL_INDEX {
            let mut block_opts = rocksdb::BlockBasedOptions::default();
            // Enable bloom filter for negative lookups on geohash prefixes
            block_opts.set_bloom_filter(10.0, false);
            opts.set_block_based_table_factory(&block_opts);
            // Use custom prefix extractor for efficient geohash range scans
            // Extracts prefix up to the 7th null byte (after geohash)
            opts.set_prefix_extractor(prefix_transform::create_spatial_index_prefix());
        }

        // Enable bloom filters for UNIQUE_INDEX CF
        // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
        // Bloom filters improve O(1) conflict detection by avoiding disk I/O for non-existent keys
        if cf_name == cf::UNIQUE_INDEX {
            let mut block_opts = rocksdb::BlockBasedOptions::default();
            // 10 bits per key gives ~1% false positive rate
            block_opts.set_bloom_filter(10.0, false);
            opts.set_block_based_table_factory(&block_opts);
        }

        cfs.push(ColumnFamilyDescriptor::new(cf_name, opts));
    }

    cfs
}

/// Open or create a RocksDB instance with all required column families using default options
///
/// For production deployments, use `open_db_with_config()` for full control over configuration.
pub fn open_db<P: AsRef<Path>>(path: P) -> Result<DB> {
    let config = config::RocksDBConfig::development().with_path(path.as_ref());
    open_db_with_config(&config)
}

/// Open or create a RocksDB instance with custom configuration
///
/// This function applies all configuration settings from the provided `RocksDBConfig`,
/// including performance tuning, compression, parallelism, and merge operators.
///
/// # Example
///
/// ```rust,no_run
/// use raisin_rocksdb::{open_db_with_config, RocksDBConfig};
///
/// let config = RocksDBConfig::production().with_path("/var/lib/raisindb");
/// let db = open_db_with_config(&config)?;
/// # Ok::<(), raisin_error::Error>(())
/// ```
pub fn open_db_with_config(config: &config::RocksDBConfig) -> Result<DB> {
    let db_opts = config.to_rocksdb_options();
    let cfs = create_column_family_descriptors();

    let db = DB::open_cf_descriptors(&db_opts, &config.path, cfs)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to open RocksDB: {}", e)))?;

    Ok(db)
}

/// Helper to get a column family handle
pub(crate) fn cf_handle<'a>(db: &'a DB, name: &str) -> Result<&'a ColumnFamily> {
    db.cf_handle(name)
        .ok_or_else(|| raisin_error::Error::storage(format!("Column family '{}' not found", name)))
}
