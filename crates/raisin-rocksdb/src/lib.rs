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
pub mod compound_state;
pub mod config;
mod constants;
pub mod embedding_provider;
mod error_ext;
pub mod fractional_index;
pub mod graph;
pub mod hnsw_transfer;
pub mod indexing;
mod jobs;
pub mod keys;
pub mod lazy_indexing;
pub mod management;
pub mod mcp_listener;
pub mod monitoring;
pub mod oauth_store;
pub mod one_time_token;
mod prefix_transform;
pub mod replication;
pub mod repositories;
pub mod secret_store;
pub mod security;
pub mod spatial;
pub mod spatial_state;
mod storage;
pub mod tantivy_transfer;
mod tombstones;
mod transaction;
pub mod vaulting;
pub mod vmount_registry;

pub use admin_user_store::AdminUserStore;
pub use api_key_store::ApiKeyStore;
pub use auth_service::{AdminClaims, AuthService};
pub use checkpoint::{CheckpointManager, CheckpointMetadata, CheckpointReceiver};
pub use config::{CompressionType, ReplicationPeerConfig, RocksDBConfig, TenantLimits};
pub use hnsw_transfer::{HnswIndexManager, HnswIndexMetadata, HnswIndexReceiver};
// Fair-share scheduling: the operator-set per-tenant weights. The server's
// admin API reads and writes these; the schedulers read the live table.
pub use jobs::fair::{
    get_weight as get_tenant_scheduling_weight, install_persisted_weights, scheduling_weights,
    set_weight as set_tenant_scheduling_weight, validate_weight, SharedWeights, WeightError,
    DEFAULT_TENANT_WEIGHT, MAX_TENANT_WEIGHT, MIN_TENANT_WEIGHT,
};
pub use jobs::{
    create_trigger_matcher,
    cron_matches,
    // Job dispatcher types
    dispatcher::DispatcherStats,
    dispatcher::JobDispatcher,
    // Flow job scheduler
    flow_scheduler::get_flow_job_scheduler,
    // Integration token-refresh dedup-key derivation (periodic driver)
    token_refresh_dedup_key,
    // Package installation callback types
    BinaryDeleteCallback,
    BinaryRetrievalCallback,
    BinaryStorageCallback,
    BinaryUploadCallback,
    // Trigger registry exports
    CachedTrigger,
    CopyTreeExecutorCallback,
    // Auto-dispatch monitor (routes registered jobs to category queues)
    DispatchingMonitor,
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
    // Worker pool (create_dispatcher for tests / embedders)
    RocksDBWorkerPool,
    ScheduledTriggerFinderCallback,
    ScheduledTriggerMatch,
    SqlExecutorCallback,
    TriggerFilters,
    TriggerMatch,
    TriggerMatcherCallback,
    TriggerRegistry,
    UnifiedJobEventHandler,
    UploadSessionCleanupHandler,
    // Job-context metadata keys carrying provenance (see `jobs/mod.rs`)
    AUTH_CONTEXT_KEY,
    ORIGIN_AGENT_KEY,
};
pub use lazy_indexing::{BuildResult, LazyIndexManager};
pub use management::{DimensionMismatch, HnswManagement, VectorRebuildStats, VerificationReport};
pub use oauth_store::RocksDbOAuthStore;
pub use one_time_token::OneTimeTokenStore;
pub use replication::OperationCapture;
pub use repositories::{
    OpLogRepository, OpLogStats, ProximityResult, RocksDBAuditRepo, RocksDBEmbeddingJobStore,
    RocksDBEmbeddingStorage, RocksDBTranslationRepository, RocksDbJobStore, SpatialIndexEntry,
    SpatialIndexRepository, SystemUpdateRepositoryImpl, TenantAIConfigRepository,
    TenantEmbeddingConfigRepository, TenantEmbeddingSpecResolver, DEFAULT_AUDIT_READ_LIMIT,
    DEFAULT_SPATIAL_MAX_ENTRIES_PER_CELL,
};

// Re-export StorageNode for internal use across modules
pub(crate) use repositories::StorageNode;
pub use tantivy_transfer::{TantivyIndexManager, TantivyIndexMetadata, TantivyIndexReceiver};

// Re-export replication handlers for external use
// Per-key queueing lock. `jobs` is private, and the wasm compile cache needs
// exactly this primitive for its single flight: a second cold caller for the
// same artifact must WAIT for the first compile rather than start its own.
// (CLAUDE.md: prefer it over another hand-rolled `HashMap<K, Arc<Mutex<()>>>`.)
pub use jobs::handlers::replication_sync::ReplicationSyncHandler;
/// Outbound MCP tool discovery, assembled by the server binary from
/// `[mcp_client]`.
pub use jobs::handlers::{McpDiscoveryDeps, McpToolDiscoveryHandler};
pub use jobs::keyed_mutex::{KeyedMutex, KeyedMutexGuard};

// The `jobs` module is private, and the server binary must reach this from
// outside: `raisin-functions` owns the wasm engine but depends on THIS crate,
// so the package installer can only learn whether an artifact is runnable if
// `main.rs` hands it a closure. Same inversion as `install_capability_probe`.
pub use jobs::wasm_validator::{
    install_wasm_validator, validate_wasm_artifact, validate_wasm_artifact_async,
    wasm_validator_installed, WasmArtifactValidator,
};

// Re-export the spatial index build handler. The `jobs` module is private, and a
// rebuild is not observable from outside the crate without either running the whole
// job system or driving the handler directly — so an integration test that has to
// prove what a REAL rebuild writes (entry revisions, precision sets) needs this.
pub use jobs::handlers::spatial_index::{SpatialBuildReport, SpatialIndexJobHandler};

// Re-exported for the same reason as the spatial handler above: whether a
// re-embed is a no-op, and whether an orphaned chunk survives it, is only
// observable by driving the real handler — the decision is made inside it, and
// the alternative is booting the whole job system.
pub use jobs::handlers::embedding::{EmbeddingJobHandler, EXTRACTED_TEXT_SPEC};

// Re-export the fulltext error counter so transport-http can render
// it without needing access to the (private) `jobs` module.
pub use jobs::handlers::{FulltextErrorCounter, FulltextErrorKind, FulltextErrorStats};

// Re-export the trigger circuit breaker types so raisin-server can build
// `TriggerSafetyConfig` from parsed TOML without needing access to the
// (private) `jobs` module.
pub use jobs::handlers::{
    BreakerTripReason, TriggerBreaker, TriggerBreakerStats, TriggerSafetyConfig,
};

// The push-notification endpoint lives in the HTTP transport but the mount
// state it stamps is owned here, so delivery health goes through the same
// seq-guarded write as every other mount-state change rather than the endpoint
// hand-rolling a second writer.
pub use jobs::handlers::virtual_mount_sync::record_push_delivery;

// The virtual-mount sync engine, for the same reason the spatial rebuild
// handler is re-exported above: `jobs` is private, and a sync is not observable
// from outside the crate without driving the real handler. The end-to-end tests
// (`tests/all/virtual_mount_sync_e2e_test.rs`) run whole syncs against a mock
// adapter — the only way to catch a failure that lives in the seam between the
// walk, the materializer and the scheduler rather than in any one of them.
//
// `ContentFetch` / `ContentTarget` ride along for the HTTP on-demand attachment
// fetch, which is the engine's only caller from outside a job.
pub use jobs::handlers::virtual_mount_sync::{
    check as virtual_mount_check, persist_mount_state, read_mount_state, AdapterError,
    AdapterInvoker, AdapterInvokerHandle, ContentFetch, ContentTarget, MountConfig, MountScope,
    MountState, VirtualMountSyncHandler, SYSTEM_WORKSPACE,
};

// Re-export the scheduled-invocation JobContext metadata keys so transport
// layers build and read invocation contexts with the same vocabulary as
// the job handler.
pub use jobs::handlers::scheduled_invocation::{
    FlowStartCallback, ScheduledInvocationHandler, META_ACTOR, META_EXTERNAL_KEY, META_INPUT,
    META_SCHEDULED_FOR, META_TARGET_PATH,
};

// Same reason for the fulltext maintenance jobs: the transport writes the
// JobContext that the worker's handler reads, so both sides must spell the
// rebuild-vs-reconcile discriminator identically.
pub use jobs::handlers::fulltext::{FULLTEXT_MODE_RECONCILE, META_FULLTEXT_MODE};
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

    // Persistent audit log
    // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0audit\0{node_id}\0{~ts_ms}{~seq}
    // Stores AuditLog entries (msgpack, field-named) for NodeTypes marked
    // `auditable`. The inverted fixed-width suffix makes a forward prefix scan
    // return a node's history newest-first without an in-memory sort.
    pub const AUDIT_LOG: &str = "audit_log";

    // Pending batch aggregator operations (durability layer)
    // Key format: {tenant}\0{repo}\0{branch}\0{queued_at_nanos_be:16}\0{uuid_bytes:16}
    // Stores PendingOpRecord (bincode) so single-op fulltext edits survive
    // process restarts before the aggregator's idle-flush window fires.
    pub const PENDING_BATCH_OPS: &str = "pending_batch_ops";

    // The secret store. THE ONLY place ciphertext lives.
    // Key format: {tenant}\0{repo}\0{branch}\0{name}\0{~rev:16}
    // `~rev` is a bitwise-NOT (descending) HLC, so a forward prefix scan returns
    // a secret's versions newest-first and older versions stay readable through a
    // rotation. `{name}` is null-free (`node/{node_id}/{field.path}` for a
    // vaulted schema field, or an operator-chosen name), which is what lets the
    // branch copier's part-2 rewrite and the `Tail` revision locator work.
    //
    // A node property never holds ciphertext — it holds a `secret://` reference.
    pub const SECRETS: &str = "secrets";
}

/// Every column family name, for callers outside this crate.
///
/// Exposed for the server's memory diagnostics, which reads RocksDB's per-CF
/// memory properties — those must cover ALL column families or the sum
/// under-reports, and this database's memory story is largely "×49".
pub fn all_column_family_names() -> Vec<&'static str> {
    all_column_families()
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
        cf::PENDING_BATCH_OPS,
        cf::AUDIT_LOG,
        cf::SECRETS,
    ]
}

/// Create column family descriptors with optimized options
///
/// **This is where every CF-scoped setting must live.** `DB::open_cf_descriptors`
/// takes CF options exclusively from these descriptors; the `Options` passed
/// alongside them supply only DB-wide settings. Anything CF-scoped placed there
/// instead is silently ignored (see [`config::RocksDBConfig::to_rocksdb_options`]).
///
/// `cache` is the single process-wide block cache — one bounded pool charged by
/// all ~49 column families, holding data, index and filter blocks alike. Per-CF
/// `BlockBasedOptions` below must ALL be built from
/// [`config::RocksDBConfig::block_table_options`] so none of them silently
/// falls back to RocksDB's private default 32MB cache.
///
/// `spatial_compaction` configures the [`spatial::SpatialPruneFilterFactory`],
/// which is attached to `cf::SPATIAL_INDEX` and to NO other column family.
pub(crate) fn create_column_family_descriptors(
    config: &config::RocksDBConfig,
    cache: &rocksdb::Cache,
    spatial_compaction: &spatial::SpatialCompactionConfig,
) -> Vec<ColumnFamilyDescriptor> {
    let mut cfs = Vec::new();
    let default_opts = config.column_family_options(cache);

    for cf_name in all_column_families() {
        let mut opts = default_opts.clone();

        // Custom prefix extractors (ORDERED_CHILDREN, SPATIAL_INDEX) come from
        // ONE table in `prefix_transform`, so code that must know whether a CF
        // has one — notably the branch fork, which scans with a SHORTER prefix
        // and therefore cannot use `prefix_iterator_cf` — reads the same list.
        if let Some(transform) = prefix_transform::custom_prefix_extractor(cf_name) {
            opts.set_prefix_extractor(transform);
        }

        // Enable bloom filters for PROPERTY_INDEX CF
        // This improves performance for negative lookups (property doesn't exist)
        // by avoiding disk I/O for non-existent keys
        if cf_name == cf::PROPERTY_INDEX {
            // 10 bits per key gives ~1% false positive rate.
            // Use ribbon filter for better space efficiency (requires RocksDB 6.15+)
            // block_opts.set_ribbon_filter(10.0);
            opts.set_block_based_table_factory(&config.block_table_options(cache, 10.0));
        }

        // Special configuration for SPATIAL_INDEX CF (geohash-based)
        // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}
        // Optimized for geohash prefix scans (proximity queries via ring expansion)
        if cf_name == cf::SPATIAL_INDEX {
            // Bloom filter for negative lookups on geohash prefixes
            opts.set_block_based_table_factory(&config.block_table_options(cache, 10.0));
            // (the geohash prefix extractor is installed above, from the shared table)
            // Prune superseded revisions and aged-out tombstones during
            // compaction. Without this the CF only ever grows: the revision is
            // part of the key, so an update writes a NEW key and RocksDB has
            // nothing to collapse. See `spatial::compaction`.
            if spatial_compaction.enabled {
                opts.set_compaction_filter_factory(spatial::SpatialPruneFilterFactory::new(
                    spatial_compaction.clone(),
                ));
            }
        }

        // Enable bloom filters for UNIQUE_INDEX CF
        // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
        // Bloom filters improve O(1) conflict detection by avoiding disk I/O for non-existent keys
        if cf_name == cf::UNIQUE_INDEX {
            // 10 bits per key gives ~1% false positive rate
            opts.set_block_based_table_factory(&config.block_table_options(cache, 10.0));
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
    // ONE cache for the whole database. `rocksdb::Cache` is a shared_ptr
    // handle and each table factory takes its own reference, so dropping this
    // binding after `open` does not free the cache.
    let cache = config.shared_block_cache();
    let cfs = create_column_family_descriptors(config, &cache, &config.spatial_compaction);

    let db = DB::open_cf_descriptors(&db_opts, &config.path, cfs)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to open RocksDB: {}", e)))?;

    Ok(db)
}

/// Helper to get a column family handle
pub(crate) fn cf_handle<'a>(db: &'a DB, name: &str) -> Result<&'a ColumnFamily> {
    db.cf_handle(name)
        .ok_or_else(|| raisin_error::Error::storage(format!("Column family '{}' not found", name)))
}
