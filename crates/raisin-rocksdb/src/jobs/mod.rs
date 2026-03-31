//! Job storage implementations for RocksDB

pub mod batch_aggregator;
pub mod cleanup;
pub mod data_store;
pub mod dispatcher;
pub mod dispatching_monitor;
pub mod event_handler;
pub mod flow_scheduler;
pub mod handlers;
pub mod index_lock;
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
    FunctionExecutionHandler, HuggingFaceModelHandler, JobHandlerRegistry, NodeChangeInfo,
    NodeCreatorCallback, NodeDeleteCleanupHandler, OpLogCompactionHandler,
    PackageCreateFromSelectionHandler, PackageExportHandler, PackageInstallHandler,
    PackageInstallMode, PackageProcessHandler, PropertyIndexJobHandler, RelationConsistencyHandler,
    ReplicationGCHandler, ReplicationSyncHandler, RestoreTreeExecutorCallback, RestoreTreeHandler,
    ResumableUploadHandler, RevisionHistoryCopyHandler, RocksDBFlowCallbacks,
    RocksDBUserNodeCreator, ScheduledTriggerHandler, SnapshotHandler, SqlExecutorCallback,
    TranslationChangeInfo, TriggerEvaluationHandler, UploadSessionCleanupHandler,
};
// Additional exports for external use (transport layer callbacks)
pub use handlers::{
    create_trigger_matcher, FlowResumeCallback, FunctionEnabledChecker, FunctionExecutionResult,
    FunctionExecutorCallback, ScheduledTriggerFinderCallback, ScheduledTriggerMatch, TriggerMatch,
    TriggerMatcherCallback,
};
pub use index_lock::{IndexKey, IndexLockManager};
pub use metadata_store::{JobMetadataStore, PersistedJobEntry};
pub use pool::RocksDBWorkerPool;
pub use trigger_registry::{CachedTrigger, TriggerFilters, TriggerRegistry};
pub use watchdog::{OnJobTimeoutFn, TimeoutWatchdog};
pub use worker::RocksDBWorker;
