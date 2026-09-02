//! Job handlers for specific job types
//!
//! This module contains handlers that implement the actual job processing logic
//! for different job types. Each handler is responsible for executing jobs
//! of a specific category (fulltext indexing, embedding generation, bulk SQL, etc.).

pub mod ai_tool_call_execution;
pub mod ai_tool_result_aggregation;
pub mod asset_processing;
pub mod auth;
pub mod bulk_sql;
pub mod calendar_expand;
pub mod compound_index;
pub mod copy_tree;
pub mod embedding;
pub mod flow_callbacks;
pub mod flow_execution;
pub mod flow_instance_execution;
pub mod fulltext;
pub mod function_execution;
pub mod huggingface_model;
pub mod integration_token_refresh;
pub mod mcp_connection_refresh;
pub mod mcp_tool_discovery;
pub mod node_delete_cleanup;
pub mod oplog_compaction;
pub mod package_create_from_selection;
pub mod package_export;
pub mod package_install;
pub mod package_process;
pub mod property_index;
pub mod relation_consistency;
pub mod replication_gc;
pub mod replication_sync;
pub mod restore_tree;
pub mod resumable_upload;
pub mod retarget_references;
pub mod revision_history_copy;
pub mod scheduled_invocation;
pub mod scheduled_trigger;
pub mod snapshot;
pub mod spatial_index;
pub mod trigger_evaluation;
pub mod trigger_matcher;
pub mod vector_clock_verification;
pub mod virtual_mount_sync;

pub use ai_tool_call_execution::{AIToolCallExecutionHandler, NodeCreatorCallback};
pub use ai_tool_result_aggregation::AIToolResultAggregationHandler;
#[allow(deprecated)]
// TODO(v0.2): AssetProcessingHandler is deprecated but still actively used
pub use asset_processing::AssetProcessingHandler;
pub use auth::{
    AuthCreateUserNodeHandler, AuthMagicLinkSendHandler, FunctionMagicLinkEmailSender,
    MagicLinkEmailSender, MagicLinkScope, RocksDBUserNodeCreator, SEND_MAGIC_LINK_PATH,
};
pub use bulk_sql::{BulkSqlHandler, SqlExecutorCallback};
pub use calendar_expand::CalendarExpandHandler;
pub use compound_index::CompoundIndexJobHandler;
pub use copy_tree::{CopyTreeExecutorCallback, CopyTreeHandler};
pub use embedding::EmbeddingJobHandler;
pub use flow_callbacks::{
    AICallerCallback as FlowAICallerCallback,
    AIStreamingCallerCallback as FlowAIStreamingCallerCallback,
    ChildrenListerCallback as FlowChildrenListerCallback, FlowEventEmitterCallback,
    FunctionExecutorCallback as FlowFunctionExecutorCallback,
    JobQueuerCallback as FlowJobQueuerCallback, NodeCreatorCallback as FlowNodeCreatorCallback,
    NodeLoaderCallback as FlowNodeLoaderCallback, NodeSaverCallback as FlowNodeSaverCallback,
    RocksDBFlowCallbacks,
};
pub use flow_execution::FlowExecutionHandler;
pub use flow_instance_execution::FlowInstanceExecutionHandler;
pub use fulltext::{
    ErrorCounterKey as FulltextErrorCounterKey, FulltextErrorCounter, FulltextErrorKind,
    FulltextErrorStats, FulltextJobHandler,
};
pub use function_execution::{
    FlowResumeCallback, FunctionEnabledChecker, FunctionExecutionHandler, FunctionExecutionResult,
    FunctionExecutorCallback,
};
pub use huggingface_model::HuggingFaceModelHandler;
pub use integration_token_refresh::{token_refresh_dedup_key, IntegrationTokenRefreshHandler};
pub use mcp_tool_discovery::{McpDiscoveryDeps, McpToolDiscoveryHandler};
pub use node_delete_cleanup::NodeDeleteCleanupHandler;
pub use oplog_compaction::OpLogCompactionHandler;
pub use package_create_from_selection::PackageCreateFromSelectionHandler;
pub use package_export::PackageExportHandler;
pub use package_install::{
    BinaryDeleteCallback, BinaryRetrievalCallback, BinaryStorageCallback, DryRunActionCounts,
    DryRunLogEntry, DryRunResult, DryRunSummary, InstallMode as PackageInstallMode,
    PackageInstallHandler,
};
pub use package_process::PackageProcessHandler;
pub use property_index::PropertyIndexJobHandler;
pub use relation_consistency::RelationConsistencyHandler;
pub use replication_gc::ReplicationGCHandler;
pub use replication_sync::ReplicationSyncHandler;
pub use restore_tree::{RestoreTreeExecutorCallback, RestoreTreeHandler};
pub use resumable_upload::{
    BinaryUploadCallback, ResumableUploadHandler, UploadSessionCleanupHandler,
};
pub use retarget_references::RetargetReferencesHandler;
pub use revision_history_copy::RevisionHistoryCopyHandler;
pub use scheduled_invocation::{FlowStartCallback, ScheduledInvocationHandler};
pub use scheduled_trigger::{
    cron_matches, ScheduledTriggerFinderCallback, ScheduledTriggerHandler, ScheduledTriggerMatch,
};
pub use snapshot::{NodeChangeInfo, SnapshotHandler, TranslationChangeInfo};
pub use spatial_index::{SpatialBuildReport, SpatialIndexJobHandler};
pub use trigger_evaluation::{
    BreakerTripReason, FilterCheckResult, TriggerBreaker, TriggerBreakerStats,
    TriggerEvaluationHandler, TriggerEvaluationReport, TriggerEvaluationResult, TriggerEventInfo,
    TriggerMatch, TriggerMatcherCallback, TriggerSafetyConfig,
};
pub use trigger_matcher::create_trigger_matcher;
pub use virtual_mount_sync::{
    AdapterError, AdapterInvoker, AdapterInvokerHandle, FunctionAdapterInvoker,
    VirtualMountSyncHandler,
};

use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::jobs::{JobContext, JobInfo};
use std::sync::Arc;

/// Registry of all job handlers
///
/// This struct holds references to all specialized handlers and provides
/// a unified interface for job dispatch.
#[allow(deprecated)] // Contains AssetProcessingHandler which is deprecated but still used
pub struct JobHandlerRegistry {
    pub fulltext: Arc<FulltextJobHandler>,
    pub embedding: Arc<EmbeddingJobHandler>,
    pub snapshot: Arc<SnapshotHandler>,
    pub replication_gc: Arc<ReplicationGCHandler>,
    pub replication_sync: Arc<ReplicationSyncHandler>,
    pub oplog_compaction: Arc<OpLogCompactionHandler>,
    pub property_index: Arc<PropertyIndexJobHandler>,
    pub compound_index: Arc<CompoundIndexJobHandler>,
    /// Spatial index build / backfill. `Option` so the (already very long)
    /// positional `new()` signature stays source-compatible; wired via
    /// [`JobHandlerRegistry::with_spatial_index`] right after construction.
    pub spatial_index: Option<Arc<SpatialIndexJobHandler>>,
    pub bulk_sql: Arc<BulkSqlHandler>,
    pub revision_history_copy: Arc<RevisionHistoryCopyHandler>,
    pub copy_tree: Arc<CopyTreeHandler>,
    pub restore_tree: Arc<RestoreTreeHandler>,
    pub node_delete_cleanup: Arc<NodeDeleteCleanupHandler>,
    pub retarget_references: Arc<RetargetReferencesHandler>,
    pub relation_consistency: Arc<RelationConsistencyHandler>,
    pub function_execution: Arc<FunctionExecutionHandler>,
    pub flow_execution: Arc<FlowExecutionHandler>,
    pub flow_instance_execution: Arc<FlowInstanceExecutionHandler>,
    pub trigger_evaluation: Arc<TriggerEvaluationHandler>,
    pub scheduled_trigger: Arc<ScheduledTriggerHandler>,
    pub scheduled_invocation: Arc<ScheduledInvocationHandler>,
    pub package_install: Arc<PackageInstallHandler<RocksDBStorage>>,
    pub package_process: Arc<PackageProcessHandler<RocksDBStorage>>,
    pub package_export: Arc<PackageExportHandler<RocksDBStorage>>,
    pub package_create_from_selection: Arc<PackageCreateFromSelectionHandler>,
    pub ai_tool_call_execution: Arc<AIToolCallExecutionHandler<RocksDBStorage>>,
    pub ai_tool_result_aggregation: Arc<AIToolResultAggregationHandler<RocksDBStorage>>,
    pub auth_create_user_node: Option<Arc<AuthCreateUserNodeHandler<RocksDBUserNodeCreator>>>,
    /// Magic-link email delivery. `None` when the server supplied no function
    /// executor, in which case a dispatched job HARD ERRORS — see the dispatch
    /// arm for why this one may not warn-and-succeed. Attached via
    /// [`JobHandlerRegistry::with_auth_magic_link_send`].
    pub auth_magic_link_send: Option<Arc<AuthMagicLinkSendHandler<FunctionMagicLinkEmailSender>>>,
    pub resumable_upload: Arc<ResumableUploadHandler<RocksDBStorage>>,
    pub upload_session_cleanup: Arc<UploadSessionCleanupHandler>,
    pub huggingface_model: Option<Arc<HuggingFaceModelHandler>>,
    pub asset_processing: Option<Arc<AssetProcessingHandler>>,
    pub integration_token_refresh: Arc<IntegrationTokenRefreshHandler>,
    pub virtual_mount_sync: Arc<VirtualMountSyncHandler>,
    /// Outbound MCP tool discovery. `None` when the server did not supply an
    /// egress policy / HTTP client, in which case discovery jobs no-op loudly
    /// rather than dialling with unknown settings.
    pub mcp_tool_discovery: Option<Arc<McpToolDiscoveryHandler>>,
    /// Derived calendar-occurrence projection rebuild. `None` leaves recurring
    /// events unexpanded, so a date-range query over a calendar sees only
    /// non-recurring events — attached explicitly so that is a deployment
    /// decision rather than an accident.
    pub calendar_expand: Option<Arc<CalendarExpandHandler>>,
}

#[allow(deprecated)] // Contains AssetProcessingHandler which is deprecated but still used
impl JobHandlerRegistry {
    /// Create a new handler registry with all handlers
    pub fn new(
        fulltext: Arc<FulltextJobHandler>,
        embedding: Arc<EmbeddingJobHandler>,
        snapshot: Arc<SnapshotHandler>,
        replication_gc: Arc<ReplicationGCHandler>,
        replication_sync: Arc<ReplicationSyncHandler>,
        oplog_compaction: Arc<OpLogCompactionHandler>,
        property_index: Arc<PropertyIndexJobHandler>,
        compound_index: Arc<CompoundIndexJobHandler>,
        bulk_sql: Arc<BulkSqlHandler>,
        revision_history_copy: Arc<RevisionHistoryCopyHandler>,
        copy_tree: Arc<CopyTreeHandler>,
        restore_tree: Arc<RestoreTreeHandler>,
        node_delete_cleanup: Arc<NodeDeleteCleanupHandler>,
        retarget_references: Arc<RetargetReferencesHandler>,
        relation_consistency: Arc<RelationConsistencyHandler>,
        function_execution: Arc<FunctionExecutionHandler>,
        flow_execution: Arc<FlowExecutionHandler>,
        flow_instance_execution: Arc<FlowInstanceExecutionHandler>,
        trigger_evaluation: Arc<TriggerEvaluationHandler>,
        scheduled_trigger: Arc<ScheduledTriggerHandler>,
        scheduled_invocation: Arc<ScheduledInvocationHandler>,
        package_install: Arc<PackageInstallHandler<RocksDBStorage>>,
        package_process: Arc<PackageProcessHandler<RocksDBStorage>>,
        package_export: Arc<PackageExportHandler<RocksDBStorage>>,
        package_create_from_selection: Arc<PackageCreateFromSelectionHandler>,
        ai_tool_call_execution: Arc<AIToolCallExecutionHandler<RocksDBStorage>>,
        ai_tool_result_aggregation: Arc<AIToolResultAggregationHandler<RocksDBStorage>>,
        auth_create_user_node: Option<Arc<AuthCreateUserNodeHandler<RocksDBUserNodeCreator>>>,
        resumable_upload: Arc<ResumableUploadHandler<RocksDBStorage>>,
        upload_session_cleanup: Arc<UploadSessionCleanupHandler>,
        huggingface_model: Option<Arc<HuggingFaceModelHandler>>,
        asset_processing: Option<Arc<AssetProcessingHandler>>,
        integration_token_refresh: Arc<IntegrationTokenRefreshHandler>,
        virtual_mount_sync: Arc<VirtualMountSyncHandler>,
    ) -> Self {
        Self {
            // Attached separately via `with_mcp_tool_discovery`: this
            // constructor already carries 30-odd positional arguments, and the
            // handler needs settings (egress policy, HTTP client) that only the
            // server binary has.
            mcp_tool_discovery: None,
            calendar_expand: None,
            auth_magic_link_send: None,
            fulltext,
            embedding,
            snapshot,
            replication_gc,
            replication_sync,
            oplog_compaction,
            property_index,
            compound_index,
            spatial_index: None,
            bulk_sql,
            revision_history_copy,
            copy_tree,
            restore_tree,
            node_delete_cleanup,
            retarget_references,
            relation_consistency,
            function_execution,
            flow_execution,
            flow_instance_execution,
            trigger_evaluation,
            scheduled_trigger,
            scheduled_invocation,
            package_install,
            package_process,
            package_export,
            package_create_from_selection,
            ai_tool_call_execution,
            ai_tool_result_aggregation,
            auth_create_user_node,
            resumable_upload,
            upload_session_cleanup,
            huggingface_model,
            asset_processing,
            integration_token_refresh,
            virtual_mount_sync,
        }
    }

    /// Attach the outbound MCP tool-discovery handler.
    ///
    /// Separate from [`Self::new`] because the handler needs the operator's
    /// egress policy and the shared HTTP client, neither of which the storage
    /// layer knows about — they are assembled in the server binary.
    pub fn with_mcp_tool_discovery(mut self, handler: Arc<McpToolDiscoveryHandler>) -> Self {
        self.mcp_tool_discovery = Some(handler);
        self
    }

    /// Attach the calendar occurrence-projection rebuild handler.
    ///
    /// Separate from `new()` for the same reason as the others: it needs the
    /// lock manager, which is assembled in the server binary.
    pub fn with_calendar_expand(mut self, handler: Arc<CalendarExpandHandler>) -> Self {
        self.calendar_expand = Some(handler);
        self
    }

    /// Attach the magic-link email send handler.
    ///
    /// Separate from `new()` for the same reason as the others: that
    /// constructor already takes ~34 positional arguments, and this handler
    /// needs the function executor callback, which the server binary assembles.
    pub fn with_auth_magic_link_send(
        mut self,
        handler: Arc<AuthMagicLinkSendHandler<FunctionMagicLinkEmailSender>>,
    ) -> Self {
        self.auth_magic_link_send = Some(handler);
        self
    }

    /// Attach the spatial index build handler.
    ///
    /// Separate from `new()` because that constructor already takes ~35 positional
    /// arguments and threading a 36th through would touch every caller for no gain.
    pub fn with_spatial_index(mut self, handler: Arc<SpatialIndexJobHandler>) -> Self {
        self.spatial_index = Some(handler);
        self
    }

    /// Dispatch a job to the appropriate handler based on job type
    ///
    /// Returns `Ok(Some(value))` if the handler produces a result (e.g., function execution),
    /// or `Ok(None)` for handlers that don't return data.
    pub async fn dispatch(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        use raisin_storage::JobType;

        // Per-repository activity accounting, so a tenant can see that its
        // background work is moving. ONE chokepoint rather than one call per
        // handler: every job in the system passes through this match, so a
        // handler added later is counted without anyone remembering to.
        //
        // The guard is RAII and held for the whole `match` below. A handler
        // that returns early, returns `Err`, parks on an open circuit breaker
        // or is aborted by the timeout watchdog still decrements — an
        // unbalanced counter here would pin a tenant's progress surface at a
        // permanent false "busy" that no later event ever clears.
        let _activity = crate::jobs::activity::JobActivityTracker::global().track(
            &job.id,
            &job.job_type,
            context,
        );

        match &job.job_type {
            JobType::FulltextIndex { .. } => {
                self.fulltext.handle_index(job, context).await.map(|_| None)
            }
            JobType::FulltextBranchCopy { .. } => self
                .fulltext
                .handle_branch_copy(job, context)
                .await
                .map(|_| None),
            JobType::FulltextBatchIndex { .. } => self
                .fulltext
                .handle_batch_index(job, context)
                .await
                .map(|_| None),
            // Operator-triggered maintenance. These run HERE rather than in a
            // detached task in the HTTP layer, so the worker that owns the job
            // also owns its terminal status — the split used to leave the job
            // contextless and marked `Failed: Job context not found` while the
            // detached task was succeeding.
            JobType::FulltextVerify
            | JobType::FulltextRebuild
            | JobType::FulltextOptimize
            | JobType::FulltextPurge => self.fulltext.handle_maintenance(job, context).await,
            JobType::EmbeddingGenerate { .. } => self
                .embedding
                .handle_generate(job, context)
                .await
                .map(|_| None),
            JobType::EmbeddingDelete { .. } => self
                .embedding
                .handle_delete(job, context)
                .await
                .map(|_| None),
            JobType::EmbeddingBranchCopy { .. } => self
                .embedding
                .handle_branch_copy(job, context)
                .await
                .map(|_| None),
            JobType::TreeSnapshot { .. } => self.snapshot.handle(job, context).await.map(|_| None),
            JobType::ReplicationGC { .. } => {
                self.replication_gc.handle(job, context).await.map(|_| None)
            }
            JobType::ReplicationSync { .. } => self
                .replication_sync
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::OpLogCompaction { .. } => self
                .oplog_compaction
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::PropertyIndexBuild { .. } => {
                self.property_index.handle(job, context).await.map(|_| None)
            }
            JobType::CompoundIndexBuild { .. } => {
                self.compound_index.handle(job, context).await.map(|_| None)
            }
            JobType::SpatialIndexBuild { .. } => match &self.spatial_index {
                Some(handler) => handler.handle(job, context).await.map(|_| None),
                // Fail loudly. Silently succeeding would leave the index unbuilt
                // while the state record claimed otherwise, which is the exact
                // silent-wrongness this subsystem is being cleaned of.
                None => Err(raisin_error::Error::storage(
                    "SpatialIndexBuild job dispatched but no spatial index handler is registered"
                        .to_string(),
                )),
            },
            JobType::BulkSql { .. } => self.bulk_sql.handle(job, context).await.map(|_| None),
            JobType::RevisionHistoryCopy { .. } => self
                .revision_history_copy
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::CopyTree { .. } => self.copy_tree.handle(job, context).await.map(|_| None),
            JobType::RestoreTree { .. } => {
                self.restore_tree.handle(job, context).await.map(|_| None)
            }
            JobType::NodeDeleteCleanup { .. } => self
                .node_delete_cleanup
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::RetargetReferences { .. } => self
                .retarget_references
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::RelationConsistencyCheck { .. } => self
                .relation_consistency
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::FunctionExecution { .. } => {
                // Function execution returns result with logs for SSE streaming
                self.function_execution.handle(job, context).await
            }
            JobType::FlowExecution { .. } => {
                // Flow execution orchestrates multiple functions and returns aggregated result
                self.flow_execution.handle(job, context).await
            }
            JobType::FlowInstanceExecution { .. } => {
                // Flow instance execution runs stateful workflows using raisin-flow-runtime
                self.flow_instance_execution.handle(job, context).await
            }
            JobType::TriggerEvaluation { .. } => {
                // Trigger evaluation returns detailed debug report
                self.trigger_evaluation.handle(job, context).await
            }
            JobType::ScheduledTriggerCheck { .. } => self
                .scheduled_trigger
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::ScheduledInvocation { .. } => {
                // One-shot scheduled invocation: routes to a function or flow
                self.scheduled_invocation.handle(job, context).await
            }
            JobType::PackageInstall { .. } => {
                // Package installation returns result with install summary
                self.package_install.handle(job, context).await
            }
            JobType::PackageProcess { .. } => {
                // Package processing extracts manifest and updates node properties
                self.package_process.handle(job, context).await
            }
            JobType::PackageExport { .. } => {
                // Package export creates a downloadable .rap file
                self.package_export.handle(job, context).await
            }
            JobType::PackageCreateFromSelection { .. } => {
                // Package creation from selected content paths
                self.package_create_from_selection
                    .handle(job, context)
                    .await
            }
            JobType::AIToolCallExecution { .. } => {
                // AIToolCall execution handles tool execution OOTB
                self.ai_tool_call_execution.handle(job, context).await
            }
            JobType::AIToolResultAggregation { .. } => {
                // AIToolResult aggregation handles parallel tool result coordination
                self.ai_tool_result_aggregation.handle(job, context).await
            }
            JobType::AuthCreateUserNode { .. } => {
                // Create user node for newly registered identity
                if let Some(ref handler) = self.auth_create_user_node {
                    handler
                        .handle(job, context)
                        .await
                        .map(|result| Some(serde_json::to_value(result).unwrap_or_default()))
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        "AuthCreateUserNode handler not configured, skipping user node creation"
                    );
                    Ok(None)
                }
            }
            JobType::AuthMagicLinkSend { .. } => match &self.auth_magic_link_send {
                Some(handler) => handler.handle(job, context).await.map(|_| None),
                // Fail loudly, unlike AuthCreateUserNode above. A user-node
                // job that no-ops leaves a user who can still log in; a
                // magic-link job that "completes" without sending leaves a
                // login the user never receives, with the job history
                // recording a success. Retries and the operator alert are the
                // only things that can recover that.
                None => Err(raisin_error::Error::storage(
                    "AuthMagicLinkSend job dispatched but no magic link send handler is registered"
                        .to_string(),
                )),
            },
            JobType::ResumableUploadComplete { .. } => {
                // Resumable upload completion reassembles chunks and creates node
                self.resumable_upload.handle(job, context).await
            }
            JobType::UploadSessionCleanup { .. } => {
                // Upload session cleanup removes expired sessions and temp files
                self.upload_session_cleanup.handle(job, context).await
            }
            JobType::HuggingFaceModelDownload { .. } => {
                // HuggingFace model download from Hub
                if let Some(ref handler) = self.huggingface_model {
                    handler.handle_download(job, context).await
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        "HuggingFace model handler not configured"
                    );
                    Ok(None)
                }
            }
            JobType::HuggingFaceModelDelete { .. } => {
                // HuggingFace model deletion from cache
                if let Some(ref handler) = self.huggingface_model {
                    handler.handle_delete(job, context).await
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        "HuggingFace model handler not configured"
                    );
                    Ok(None)
                }
            }
            JobType::IntegrationTokenRefresh { .. } => self
                .integration_token_refresh
                .handle(job, context)
                .await
                .map(|_| None),
            JobType::VirtualMountSyncCheck { .. }
            | JobType::VirtualMountSync { .. }
            | JobType::VirtualMountWriteDrain { .. }
            | JobType::VirtualMountWriteReconcile { .. }
            | JobType::VirtualMountSubscriptionRenew { .. } => {
                self.virtual_mount_sync.handle(job, context).await
            }
            JobType::CalendarOccurrenceRebuild { .. } => {
                if let Some(ref handler) = self.calendar_expand {
                    handler.handle(job, context).await
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        "Calendar occurrence expander not configured; recurring events \
                         will not appear in date-range queries"
                    );
                    Ok(None)
                }
            }
            JobType::McpToolDiscovery { .. } | JobType::McpDiscoveryCheck { .. } => {
                if let Some(ref handler) = self.mcp_tool_discovery {
                    handler.handle(job, context).await
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        "MCP tool discovery handler not configured"
                    );
                    Ok(None)
                }
            }
            JobType::AssetProcessing { .. } => {
                // Asset processing (PDF text extraction, image embeddings, captions)
                if let Some(ref handler) = self.asset_processing {
                    handler.handle(job, context).await
                } else {
                    tracing::warn!(
                        job_id = %job.id,
                        "Asset processing handler not configured"
                    );
                    Ok(None)
                }
            }
            _ => {
                // Other job types will be handled by management operations
                Err(raisin_error::Error::Validation(format!(
                    "Unsupported job type for worker dispatch: {}",
                    job.job_type
                )))
            }
        }
    }
}
