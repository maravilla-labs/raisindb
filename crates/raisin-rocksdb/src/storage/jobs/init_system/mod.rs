//! Job system initialization
//!
//! Contains the `init_job_system` method which creates all job handlers,
//! sets up the three-pool worker system, event handler, and background
//! maintenance tasks.
//!
//! Submodules:
//! - `indexing_handlers`: Fulltext, embedding, property index, compound index
//! - `replication_handlers`: Snapshot, replication GC, sync, oplog compaction
//! - `package_handlers`: Package install, process, export, create-from-selection
//! - `flow_handlers`: Function/flow execution, triggers, bulk SQL, tree ops
//! - `ai_handlers`: AI tool calls, auth user nodes, uploads, asset processing
//! - `worker_setup`: Worker pool, batch aggregator, event handler, background tasks

mod ai_handlers;
mod flow_handlers;
mod indexing_handlers;
mod package_handlers;
mod replication_handlers;
mod worker_setup;

use crate::config::JobPoolsConfig;
use crate::jobs::{
    BinaryRetrievalCallback, BinaryStorageCallback, BinaryUploadCallback, FlowAICallerCallback,
    FlowAIStreamingCallerCallback, FlowChildrenListerCallback, FlowFunctionExecutorCallback,
    FlowJobQueuerCallback, FlowNodeCreatorCallback, FlowNodeLoaderCallback, FlowNodeSaverCallback,
    FunctionEnabledChecker, FunctionExecutorCallback, JobHandlerRegistry, NodeCreatorCallback,
    RocksDBWorkerPool, ScheduledTriggerFinderCallback, SqlExecutorCallback,
};
use crate::storage::RocksDBStorage;
use raisin_error::Result;
use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::tantivy_engine::TantivyIndexingEngine;
use raisin_storage::jobs::{JobCategory, WorkerPool};
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

impl RocksDBStorage {
    /// Initialize the background job system with three-pool isolation
    ///
    /// Creates all job handlers, sets up the worker pools (Realtime, Background,
    /// System), event handler, and background maintenance tasks.
    ///
    /// # Arguments
    ///
    /// * `runtimes` - Per-category tokio runtime handles for thread isolation
    /// * `pools_config` - Per-category pool configuration (workers, concurrency)
    ///
    /// # Returns
    ///
    /// A tuple of `(Arc<RocksDBWorkerPool>, CancellationToken)`.
    pub async fn init_job_system(
        self: Arc<Self>,
        tantivy_engine: Arc<TantivyIndexingEngine>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        sql_executor: Option<SqlExecutorCallback>,
        copy_tree_executor: Option<crate::jobs::CopyTreeExecutorCallback>,
        restore_tree_executor: Option<crate::jobs::RestoreTreeExecutorCallback>,
        function_executor: Option<FunctionExecutorCallback>,
        function_enabled_checker: Option<FunctionEnabledChecker>,
        scheduled_trigger_finder: Option<ScheduledTriggerFinderCallback>,
        binary_retrieval: Option<BinaryRetrievalCallback>,
        binary_storage: Option<BinaryStorageCallback>,
        binary_upload: Option<BinaryUploadCallback>,
        flow_node_loader: Option<FlowNodeLoaderCallback>,
        flow_node_saver: Option<FlowNodeSaverCallback>,
        flow_node_creator: Option<FlowNodeCreatorCallback>,
        flow_job_queuer: Option<FlowJobQueuerCallback>,
        flow_ai_caller: Option<FlowAICallerCallback>,
        flow_ai_streaming_caller: Option<FlowAIStreamingCallerCallback>,
        flow_function_executor: Option<FlowFunctionExecutorCallback>,
        flow_children_lister: Option<FlowChildrenListerCallback>,
        ai_tool_call_node_creator: Option<NodeCreatorCallback>,
        runtimes: HashMap<JobCategory, tokio::runtime::Handle>,
        pools_config: JobPoolsConfig,
    ) -> Result<(Arc<RocksDBWorkerPool>, CancellationToken)> {
        if !self.config.background_jobs_enabled {
            return Err(raisin_error::Error::Validation(
                "Background jobs are not enabled in configuration".to_string(),
            ));
        }

        tracing::info!("Initializing background job system with three-pool isolation");

        let master_key = Self::get_master_encryption_key()?;

        // Create job dispatcher — per-category channel sets
        let (dispatcher, receivers) = RocksDBWorkerPool::create_dispatcher();
        self.set_job_dispatcher(dispatcher.clone());

        // Register auto-dispatch monitor (routes by category)
        let dispatching_monitor =
            Arc::new(crate::jobs::DispatchingMonitor::new(dispatcher.clone()));
        self.job_registry
            .monitors()
            .add_monitor(dispatching_monitor)
            .await;

        // --- Create all job handlers ---

        let fulltext_handler =
            indexing_handlers::create_fulltext_handler(self.clone(), tantivy_engine.clone());
        let embedding_handler = indexing_handlers::create_embedding_handler(
            self.clone(),
            hnsw_engine.clone(),
            master_key,
        );
        let property_index_handler = indexing_handlers::create_property_index_handler(&self);
        let compound_index_handler = indexing_handlers::create_compound_index_handler(&self);

        let snapshot_handler = replication_handlers::create_snapshot_handler(&self);
        let replication_gc_handler = replication_handlers::create_replication_gc_handler(&self);
        let replication_sync_handler = replication_handlers::create_replication_sync_handler(&self);
        let oplog_compaction_handler = replication_handlers::create_oplog_compaction_handler(&self);

        let bulk_sql_handler = flow_handlers::create_bulk_sql_handler(sql_executor);
        let (copy_tree_handler, restore_tree_handler, revision_history_copy_handler) =
            flow_handlers::create_tree_handlers(&self, copy_tree_executor, restore_tree_executor);

        // Clone function_executor for AI handler before consuming
        let function_executor_for_ai = function_executor.clone();

        let function_execution_handler = flow_handlers::create_function_execution_handler(
            self.job_registry.clone(),
            self.job_data_store.clone(),
            function_executor.as_ref(),
            function_enabled_checker.as_ref(),
        );

        let flow_execution_handler = flow_handlers::create_flow_execution_handler(
            self.job_registry.clone(),
            self.job_data_store.clone(),
            function_executor,
            function_enabled_checker,
        );

        let flow_instance_execution_handler = flow_handlers::create_flow_instance_execution_handler(
            flow_node_loader,
            flow_node_saver,
            flow_node_creator,
            flow_job_queuer,
            flow_ai_caller,
            flow_ai_streaming_caller,
            flow_function_executor,
            flow_children_lister,
        );

        let trigger_evaluation_handler = flow_handlers::create_trigger_evaluation_handler(
            self.clone(),
            self.job_registry.clone(),
            self.job_data_store.clone(),
            dispatcher.clone(),
        );

        let scheduled_trigger_handler = flow_handlers::create_scheduled_trigger_handler(
            self.job_registry.clone(),
            self.job_data_store.clone(),
            dispatcher.clone(),
            scheduled_trigger_finder,
        );

        let package_install_handler = package_handlers::create_package_install_handler(
            self.clone(),
            self.job_registry.clone(),
            binary_retrieval.as_ref(),
            binary_storage.as_ref(),
        );
        let package_process_handler = package_handlers::create_package_process_handler(
            self.clone(),
            self.job_registry.clone(),
            binary_retrieval.as_ref(),
            binary_storage.as_ref(),
        );
        let package_export_handler = package_handlers::create_package_export_handler(
            self.clone(),
            self.job_registry.clone(),
            binary_storage.as_ref(),
        );
        let package_create_from_selection_handler =
            package_handlers::create_package_create_from_selection_handler(
                self.clone(),
                self.job_registry.clone(),
                binary_retrieval.as_ref(),
                binary_storage.as_ref(),
            );

        let ai_tool_call_execution_handler = ai_handlers::create_ai_tool_call_execution_handler(
            self.clone(),
            function_executor_for_ai,
            ai_tool_call_node_creator.as_ref(),
        );
        let ai_tool_result_aggregation_handler =
            ai_handlers::create_ai_tool_result_aggregation_handler(
                self.clone(),
                ai_tool_call_node_creator,
            );

        let node_delete_cleanup_handler = ai_handlers::create_node_delete_cleanup_handler(&self);
        let relation_consistency_handler = ai_handlers::create_relation_consistency_handler(&self);
        let auth_create_user_node_handler =
            ai_handlers::create_auth_user_node_handler(self.clone());

        let resumable_upload_handler = ai_handlers::create_resumable_upload_handler(
            self.clone(),
            self.job_registry.clone(),
            binary_upload,
        );
        let upload_session_cleanup_handler = ai_handlers::create_upload_session_cleanup_handler();
        let huggingface_model_handler = ai_handlers::create_huggingface_model_handler();

        #[allow(deprecated)]
        let asset_processing_handler =
            ai_handlers::create_asset_processing_handler(self.clone(), binary_retrieval.as_ref());

        // --- Assemble handler registry ---

        let handlers = Arc::new(JobHandlerRegistry::new(
            fulltext_handler,
            embedding_handler,
            snapshot_handler,
            replication_gc_handler,
            replication_sync_handler,
            oplog_compaction_handler,
            property_index_handler,
            compound_index_handler,
            bulk_sql_handler,
            revision_history_copy_handler,
            copy_tree_handler,
            restore_tree_handler,
            node_delete_cleanup_handler,
            relation_consistency_handler,
            function_execution_handler,
            flow_execution_handler,
            flow_instance_execution_handler,
            trigger_evaluation_handler,
            scheduled_trigger_handler,
            package_install_handler,
            package_process_handler,
            package_export_handler,
            package_create_from_selection_handler,
            ai_tool_call_execution_handler.clone(),
            ai_tool_result_aggregation_handler,
            auth_create_user_node_handler,
            resumable_upload_handler,
            upload_session_cleanup_handler,
            huggingface_model_handler,
            asset_processing_handler,
        ));

        // --- Set up three-pool worker system ---

        let worker_pool = worker_setup::create_multi_pool(
            self.clone(),
            self.job_registry.clone(),
            self.job_data_store.clone(),
            handlers,
            dispatcher.clone(),
            receivers,
            runtimes,
            &pools_config,
        );

        let (batch_aggregator, _batch_shutdown) = worker_setup::start_batch_aggregator(
            self.job_registry.clone(),
            self.job_data_store.clone(),
            dispatcher.clone(),
        );

        worker_setup::subscribe_event_handler(
            self.clone(),
            self.job_registry.clone(),
            self.job_data_store.clone(),
            dispatcher.clone(),
            batch_aggregator,
        );

        let restore_stats = worker_setup::restore_and_dispatch_jobs(&self, &dispatcher).await?;

        worker_pool.start().await?;

        worker_setup::start_background_tasks(&self, ai_tool_call_execution_handler.clone());

        tracing::info!(
            realtime_workers = pools_config.realtime.dispatcher_workers,
            realtime_max_handlers = pools_config.realtime.max_concurrent_handlers,
            background_workers = pools_config.background.dispatcher_workers,
            background_max_handlers = pools_config.background.max_concurrent_handlers,
            system_workers = pools_config.system.dispatcher_workers,
            system_max_handlers = pools_config.system.max_concurrent_handlers,
            restored_jobs = restore_stats.restored,
            reset_running_jobs = restore_stats.reset_running,
            failed_restore = restore_stats.failed_to_restore,
            "Background job system initialized with three-pool isolation"
        );

        let shutdown_token = CancellationToken::new();
        Ok((worker_pool, shutdown_token))
    }
}
