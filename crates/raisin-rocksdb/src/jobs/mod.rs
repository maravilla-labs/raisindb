//! Job storage implementations for RocksDB

/// `JobContext.metadata` key carrying the agent marker of whatever caused the
/// job — the `agent` stamped on the node event that enqueued it, or the trigger
/// that fired. Jobs compose their own identity on top of it (see
/// `raisin_models::auth::agent_identity::with_origin`), which is what makes an
/// agent run traceable back to the trigger that started it.
pub const ORIGIN_AGENT_KEY: &str = "origin_agent";

/// `JobContext.metadata` key holding a serialized `AuthContext` for the job to
/// run under. Written by the HTTP async-invoke handler and by trigger
/// evaluation; read by `handlers/function_execution.rs`.
pub const AUTH_CONTEXT_KEY: &str = "auth_context";

/// `JobContext.metadata` key holding the raw actor id (`metadata["actor"]` off
/// the `NodeEvent` that fired a trigger — see `transaction/commit/events.rs`)
/// of whoever's write caused this job. Distinct from `ORIGIN_AGENT_KEY`, which
/// is a provenance MARKER string (`trigger:/triggers/t`), and from
/// `AUTH_CONTEXT_KEY`, which is a full serialized system context for the
/// trigger's own writes: this is a bare, resolvable user id, carried so an
/// `execution_context: "user"` agent invoked further downstream (e.g. inside a
/// triggered flow) can run AS that human rather than as System. Absent for
/// event classes with no human behind them (a timer trigger) or when the
/// write's actor resolved to `"anonymous"`/`"system"`.
pub const TRIGGERING_ACTOR_KEY: &str = "triggering_actor";

pub mod activity;
pub mod batch_aggregator;
pub mod circuit_breaker;
pub mod cleanup;
pub mod data_store;
pub mod dispatcher;
pub mod dispatching_monitor;
pub mod event_handler;
pub mod fair;
pub mod flow_instance_lock;
pub mod flow_scheduler;
pub mod handlers;
pub mod index_lock;
pub mod keyed_mutex;
pub mod metadata_store;
pub mod pool;
pub mod trigger_registry;
pub mod wasm_validator;
pub mod watchdog;
pub mod worker;

pub use activity::{install_publisher as install_job_activity_publisher, JobActivityTracker};
pub use batch_aggregator::{BatchAggregatorConfig, BatchIndexAggregator};
pub use circuit_breaker::{BreakerPolicy, BreakerState, BreakerStatus, CircuitBreakerRegistry};
pub use cleanup::JobCleanupTask;
pub use data_store::JobDataStore;
pub use dispatching_monitor::DispatchingMonitor;
pub use event_handler::UnifiedJobEventHandler;
pub use handlers::{
    AIToolCallExecutionHandler, AIToolResultAggregationHandler, AssetProcessingHandler,
    AuthCreateUserNodeHandler, AuthMagicLinkSendHandler, BinaryDeleteCallback,
    BinaryRetrievalCallback, BinaryStorageCallback, BinaryUploadCallback, BulkSqlHandler,
    CompoundIndexJobHandler, CopyTreeExecutorCallback, CopyTreeHandler, DryRunActionCounts,
    DryRunLogEntry, DryRunResult, DryRunSummary, EmbeddingJobHandler, FlowAICallerCallback,
    FlowAIStreamingCallerCallback, FlowChildrenListerCallback, FlowEventEmitterCallback,
    FlowExecutionHandler, FlowFunctionExecutorCallback, FlowInstanceExecutionHandler,
    FlowJobQueuerCallback, FlowNodeCreatorCallback, FlowNodeLoaderCallback, FlowNodeSaverCallback,
    FulltextJobHandler, FunctionExecutionHandler, FunctionMagicLinkEmailSender,
    HuggingFaceModelHandler, IntegrationTokenRefreshHandler, JobHandlerRegistry, McpDiscoveryDeps,
    McpToolDiscoveryHandler, NodeChangeInfo, NodeCreatorCallback, NodeDeleteCleanupHandler,
    OpLogCompactionHandler, PackageCreateFromSelectionHandler, PackageExportHandler,
    PackageInstallHandler, PackageInstallMode, PackageProcessHandler, PropertyIndexJobHandler,
    RelationConsistencyHandler, ReplicationGCHandler, ReplicationSyncHandler,
    RestoreTreeExecutorCallback, RestoreTreeHandler, ResumableUploadHandler,
    RetargetReferencesHandler, RevisionHistoryCopyHandler, RocksDBFlowCallbacks,
    RocksDBUserNodeCreator, ScheduledInvocationHandler, ScheduledTriggerHandler, SnapshotHandler,
    SqlExecutorCallback, TranslationChangeInfo, TriggerBreaker, TriggerBreakerStats,
    TriggerEvaluationHandler, TriggerSafetyConfig, UploadSessionCleanupHandler,
    VirtualMountSyncHandler,
};
// Additional exports for external use (transport layer callbacks)
pub use flow_instance_lock::{FlowInstanceBusy, FlowInstanceLease, FlowInstanceLockManager};
pub use handlers::{
    create_trigger_matcher, cron_matches, token_refresh_dedup_key, FlowResumeCallback,
    FlowStartCallback, FunctionEnabledChecker, FunctionExecutionResult, FunctionExecutorCallback,
    ScheduledTriggerFinderCallback, ScheduledTriggerMatch, TriggerMatch, TriggerMatcherCallback,
};
pub use index_lock::{IndexKey, IndexLockManager};
pub use keyed_mutex::{KeyedMutex, KeyedMutexGuard};
pub use metadata_store::{JobMetadataStore, PersistedJobEntry};
pub use pool::RocksDBWorkerPool;
pub use trigger_registry::{CachedTrigger, TriggerFilters, TriggerRegistry};
pub use wasm_validator::{
    install_wasm_validator, validate_wasm_artifact, validate_wasm_artifact_async,
    wasm_validator_installed, WasmArtifactValidator,
};
pub use watchdog::{OnJobTimeoutFn, TimeoutWatchdog};
pub use worker::RocksDBWorker;
