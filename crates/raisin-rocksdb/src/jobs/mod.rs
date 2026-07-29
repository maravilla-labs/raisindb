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

pub mod batch_aggregator;
pub mod cleanup;
pub mod data_store;
pub mod dispatcher;
pub mod dispatching_monitor;
pub mod event_handler;
pub mod flow_instance_lock;
pub mod flow_scheduler;
pub mod handlers;
pub mod index_lock;
pub mod keyed_mutex;
pub mod metadata_store;
pub mod pool;
pub mod trigger_registry;
pub mod watchdog;
pub mod worker;

pub use batch_aggregator::{BatchAggregatorConfig, BatchIndexAggregator};
pub use cleanup::JobCleanupTask;
pub use data_store::JobDataStore;
pub use dispatching_monitor::DispatchingMonitor;
pub use event_handler::UnifiedJobEventHandler;
pub use handlers::{
    AIToolCallExecutionHandler, AIToolResultAggregationHandler, AssetProcessingHandler,
    AuthCreateUserNodeHandler, BinaryRetrievalCallback, BinaryStorageCallback,
    BinaryUploadCallback, BulkSqlHandler, CompoundIndexJobHandler, CopyTreeExecutorCallback,
    CopyTreeHandler, DryRunActionCounts, DryRunLogEntry, DryRunResult, DryRunSummary,
    EmbeddingJobHandler, FlowAICallerCallback, FlowAIStreamingCallerCallback,
    FlowChildrenListerCallback, FlowEventEmitterCallback, FlowExecutionHandler,
    FlowFunctionExecutorCallback, FlowInstanceExecutionHandler, FlowJobQueuerCallback,
    FlowNodeCreatorCallback, FlowNodeLoaderCallback, FlowNodeSaverCallback, FulltextJobHandler,
    FunctionExecutionHandler, HuggingFaceModelHandler, IntegrationTokenRefreshHandler,
    JobHandlerRegistry, NodeChangeInfo, NodeCreatorCallback, NodeDeleteCleanupHandler,
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
pub use watchdog::{OnJobTimeoutFn, TimeoutWatchdog};
pub use worker::RocksDBWorker;
