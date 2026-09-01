// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Callback type definitions for the execution module.
//!
//! These types are re-exported from raisin_rocksdb to ensure compatibility
//! with the job system callbacks.

use std::sync::Arc;

use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;

// Re-export callback types from raisin_rocksdb for compatibility
pub use raisin_rocksdb::{
    BinaryRetrievalCallback, FunctionEnabledChecker, FunctionExecutionResult,
    FunctionExecutorCallback, SqlExecutorCallback,
};

/// The process-wide HTTP client for function execution.
///
/// `reqwest::Client` is already an `Arc` internally — cloning is cheap and
/// shares the connection pool, which is the whole point. CONSTRUCTING one is
/// not cheap: it builds a fresh pool and loads the TLS root store.
///
/// Six sites used to call `reqwest::Client::new()` independently, several of
/// them per request rather than at startup: the MCP path built one for the
/// caller backend, one for the discovery backend, and a third per tool
/// invocation — three per `tools/call`, whether or not the tool made a single
/// HTTP request. Beyond the setup cost, a client discarded after one call can
/// never reuse a connection, so every outbound request paid a fresh TCP + TLS
/// handshake too.
pub fn shared_http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

/// Bundle of all dependencies needed for function execution.
///
/// This struct is created once at startup and passed to `ExecutionProvider`
/// to create the execution callbacks. It bundles storage, binary storage,
/// indexing engines, HTTP client, and AI configuration.
///
/// # Type Parameters
/// - `S`: Storage implementation (e.g., `RocksDBStorage`)
/// - `B`: Binary storage implementation (e.g., `FilesystemBinaryStorage`)
/// Late-bound access to the virtual-mount sync engine. See
/// [`ExecutionDependencies::mount_content`] for why this is a resolver.
pub type MountContentResolver =
    Arc<dyn Fn() -> Option<Arc<raisin_rocksdb::VirtualMountSyncHandler>> + Send + Sync>;

pub struct ExecutionDependencies<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    /// Storage for node/workspace operations
    pub storage: Arc<S>,

    /// Binary storage for loading asset code
    pub binary_storage: Arc<B>,

    /// Indexing engine for SQL queries (optional - SQL won't work without it)
    pub indexing_engine: Option<Arc<raisin_indexer::TantivyIndexingEngine>>,

    /// HNSW engine for vector operations (optional - vector search won't work without it)
    pub hnsw_engine: Option<Arc<raisin_hnsw::HnswIndexingEngine>>,

    /// HTTP client for external requests.
    ///
    /// Prefer [`shared_http_client`] over `reqwest::Client::new()`: building one
    /// constructs a fresh connection pool and TLS root store, and this struct is
    /// assembled per REQUEST on the MCP and WS paths — three times per
    /// `tools/call` — so a per-call client both costs real latency and throws
    /// away connection reuse for the tools that do make HTTP calls.
    pub http_client: reqwest::Client,

    /// AI config store for tenant-scoped AI configuration (optional - AI won't work without it)
    pub ai_config_store: Option<Arc<dyn raisin_ai::TenantAIConfigStore>>,

    /// Job registry for queuing function execution jobs (optional - nested function calls won't work without it)
    pub job_registry: Option<Arc<raisin_storage::jobs::JobRegistry>>,

    /// Job data store for storing job context (optional - nested function calls won't work without it)
    pub job_data_store: Option<Arc<raisin_rocksdb::JobDataStore>>,

    /// Atomic locks / inventory manager (optional - `raisin.locks`/`raisin.inventory`
    /// won't work without it)
    pub lock_manager: Option<raisin_locks::LockManagerHandle>,

    /// Secret store for `raisin.secrets.*` (optional).
    ///
    /// `None` when the deployment has no master keyring configured, in which
    /// case the binding reports the subsystem as unconfigured. Note that a
    /// PRESENT store is not itself a grant — every call is gated by the
    /// function's `SecretPolicy`, which denies by default.
    pub secret_store: Option<Arc<raisin_rocksdb::secret_store::SecretStore>>,

    /// Identity repository for `raisin.identities.*` (optional).
    ///
    /// `None` on paths without a RocksDB handle, in which case the binding
    /// reports the subsystem as unavailable. A PRESENT repository is not a
    /// grant — every call is gated by the function's `IdentityPolicy`, which
    /// denies by default.
    pub identity_repo: Option<Arc<raisin_rocksdb::repositories::IdentityRepository>>,

    /// Resolves the virtual-mount sync engine, for fetching a mounted asset's
    /// bytes when a function reads them and the local cache is empty.
    ///
    /// A RESOLVER and not the handler itself, because of construction order:
    /// these dependencies are assembled before `init_job_system`, which is what
    /// builds the sync engine and publishes it. Capturing the handle here would
    /// capture `None` permanently, and mounted content would never load — a
    /// silent, permanent failure of the kind that looks like a missing file.
    ///
    /// Returning `None` at call time is the honest "no sync engine on this
    /// deployment", which the fetch reports as such.
    pub mount_content: Option<MountContentResolver>,

    /// Schema statistics cache for SQL planner selectivity estimation.
    ///
    /// Without it, every `raisin.sql.query()` call re-runs two full listings (all
    /// NodeTypes and all Archetypes) just to read their counts — the HTTP, WS and
    /// pgwire paths all wire this, and the function path was the one that did not.
    pub schema_stats_cache: Option<raisin_core::SharedSchemaStatsCache>,
}

/// Configuration for function execution
#[derive(Debug, Clone)]
pub struct FunctionExecutionConfig {
    /// Timeout for function execution in milliseconds (default: 30_000 = 30s)
    pub timeout_ms: u64,

    /// Memory limit in bytes (default: 128MB)
    pub memory_limit_bytes: u64,

    /// Network policy for HTTP requests
    pub network_policy: crate::types::NetworkPolicy,

    /// Workspace where functions are stored (default: "functions")
    pub functions_workspace: String,
}

impl Default for FunctionExecutionConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            memory_limit_bytes: 128 * 1024 * 1024, // 128MB
            network_policy: crate::types::NetworkPolicy::default(),
            functions_workspace: "functions".to_string(),
        }
    }
}

/// Bundle of all execution callbacks for the job system
///
/// This struct collects all the callbacks needed by `init_job_system()`.
/// Use `ExecutionProvider::create_callbacks()` to create instances.
#[derive(Clone, Default)]
pub struct ExecutionCallbacks {
    /// SQL executor callback for bulk SQL jobs
    pub sql_executor: Option<SqlExecutorCallback>,
    /// Function executor callback for serverless functions
    pub function_executor: Option<FunctionExecutorCallback>,
    /// Function enabled checker callback
    pub function_enabled_checker: Option<FunctionEnabledChecker>,
    /// Binary retrieval callback for packages
    pub binary_retrieval: Option<BinaryRetrievalCallback>,
}

/// Execution mode for the callback provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    /// Debug mode: Print events in color, return stub results
    /// Use this during development and testing
    #[default]
    Debug,
    /// Production mode: Full execution with real operations
    /// Not yet implemented - will be added after debug mode is verified
    Production,
}

/// Context for function execution containing parsed event information
///
/// This struct is populated from the `flow_input` in the function execution input.
/// It provides a structured view of the triggering event and the affected node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionContext {
    /// The type of event that triggered the function (Created, Updated, Deleted, etc.)
    pub event_type: crate::types::NodeEventKind,
    /// The ID of the node that triggered the event
    pub node_id: String,
    /// The node type (e.g., "raisin:AIMessage")
    pub node_type: String,
    /// The workspace where the event occurred
    pub workspace: String,
    /// The execution ID for this function invocation
    pub execution_id: String,
    /// The node data (fetched from storage)
    pub node: Option<raisin_models::nodes::Node>,
}
