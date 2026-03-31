//! Flow and function execution job handler construction
//!
//! Creates handlers for function execution, flow execution,
//! flow instance execution, trigger evaluation, and scheduled triggers.

use std::sync::Arc;

use crate::jobs::JobDataStore;
use crate::jobs::{
    dispatcher::JobDispatcher, FlowAICallerCallback, FlowAIStreamingCallerCallback,
    FlowChildrenListerCallback, FlowFunctionExecutorCallback, FlowJobQueuerCallback,
    FlowNodeCreatorCallback, FlowNodeLoaderCallback, FlowNodeSaverCallback, FlowResumeCallback,
    FunctionEnabledChecker, FunctionExecutorCallback, ScheduledTriggerFinderCallback,
    SqlExecutorCallback,
};
use crate::storage::RocksDBStorage;
use raisin_storage::jobs::JobRegistry;

use super::super::flow_events::convert_flow_event;

/// Create the flow resume callback that queues flow resume jobs after function execution
pub fn create_flow_resume_callback(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
) -> FlowResumeCallback {
    Arc::new(
        move |instance_id: String,
              result: serde_json::Value,
              tenant_id: String,
              repo_id: String,
              branch: String| {
            let job_registry = job_registry.clone();
            let job_data_store = job_data_store.clone();
            Box::pin(async move {
                use raisin_hlc::HLC;
                use raisin_storage::jobs::{JobContext, JobType};

                let job = JobType::FlowInstanceExecution {
                    instance_id: instance_id.clone(),
                    execution_type: "resume".to_string(),
                    resume_reason: Some("function_result".to_string()),
                };

                let job_id = job_registry
                    .register_job(job, Some(tenant_id.clone()), None, None, None)
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!(
                            "Failed to register flow resume job: {}",
                            e
                        ))
                    })?;

                let mut metadata = std::collections::HashMap::new();
                metadata.insert("function_result".to_string(), result);

                let context = JobContext {
                    tenant_id,
                    repo_id,
                    branch,
                    workspace_id: "raisin:system".to_string(),
                    revision: HLC::new(0, 0),
                    metadata,
                };

                job_data_store.put(&job_id, &context).map_err(|e| {
                    raisin_error::Error::Backend(format!(
                        "Failed to store flow resume job context: {}",
                        e
                    ))
                })?;

                tracing::info!(
                    job_id = %job_id,
                    instance_id = %instance_id,
                    "Queued flow resume job after function execution"
                );

                Ok(())
            })
        },
    )
}

/// Create the function execution handler
pub fn create_function_execution_handler(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    function_executor: Option<&FunctionExecutorCallback>,
    function_enabled_checker: Option<&FunctionEnabledChecker>,
) -> Arc<crate::jobs::FunctionExecutionHandler> {
    let mut builder =
        crate::jobs::FunctionExecutionHandler::new().with_job_registry(job_registry.clone());
    if let Some(executor) = function_executor {
        builder = builder.with_executor(executor.clone());
    }
    if let Some(checker) = function_enabled_checker {
        builder = builder.with_enabled_checker(checker.clone());
    }

    let flow_resume = create_flow_resume_callback(job_registry, job_data_store);
    builder = builder.with_flow_resumer(flow_resume);

    Arc::new(builder)
}

/// Create the flow execution handler
pub fn create_flow_execution_handler(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    function_executor: Option<FunctionExecutorCallback>,
    function_enabled_checker: Option<FunctionEnabledChecker>,
) -> Arc<crate::jobs::FlowExecutionHandler> {
    let mut builder = crate::jobs::FlowExecutionHandler::new(job_registry, job_data_store);
    if let Some(executor) = function_executor {
        builder = builder.with_executor(executor);
    }
    if let Some(checker) = function_enabled_checker {
        builder = builder.with_enabled_checker(checker);
    }
    Arc::new(builder)
}

/// Create the flow instance execution handler for stateful workflows
pub fn create_flow_instance_execution_handler(
    flow_node_loader: Option<FlowNodeLoaderCallback>,
    flow_node_saver: Option<FlowNodeSaverCallback>,
    flow_node_creator: Option<FlowNodeCreatorCallback>,
    flow_job_queuer: Option<FlowJobQueuerCallback>,
    flow_ai_caller: Option<FlowAICallerCallback>,
    flow_ai_streaming_caller: Option<FlowAIStreamingCallerCallback>,
    flow_function_executor: Option<FlowFunctionExecutorCallback>,
    flow_children_lister: Option<FlowChildrenListerCallback>,
) -> Arc<crate::jobs::FlowInstanceExecutionHandler> {
    let mut builder = crate::jobs::FlowInstanceExecutionHandler::new();
    if let Some(loader) = flow_node_loader {
        builder = builder.with_node_loader(loader);
    }
    if let Some(saver) = flow_node_saver {
        builder = builder.with_node_saver(saver);
    }
    if let Some(creator) = flow_node_creator {
        builder = builder.with_node_creator(creator);
    }
    if let Some(queuer) = flow_job_queuer {
        builder = builder.with_job_queuer(queuer);
    }
    if let Some(caller) = flow_ai_caller {
        builder = builder.with_ai_caller(caller);
    }
    if let Some(caller) = flow_ai_streaming_caller {
        builder = builder.with_ai_streaming_caller(caller);
    }
    if let Some(executor) = flow_function_executor {
        builder = builder.with_function_executor(executor);
    }
    if let Some(lister) = flow_children_lister {
        builder = builder.with_children_lister(lister);
    }

    // Wire up flow event emitter to the global broadcaster for SSE streaming
    let event_emitter: crate::jobs::FlowEventEmitterCallback = Arc::new(
        |instance_id: String, event: raisin_flow_runtime::types::FlowExecutionEvent| {
            Box::pin(async move {
                let flow_event = convert_flow_event(event);
                raisin_storage::jobs::global_flow_broadcaster().emit(&instance_id, flow_event);
                Ok(())
            })
        },
    );
    builder = builder.with_event_emitter(event_emitter);

    Arc::new(builder)
}

/// Create the trigger evaluation handler
pub fn create_trigger_evaluation_handler(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    dispatcher: Arc<JobDispatcher>,
) -> Arc<crate::jobs::TriggerEvaluationHandler> {
    let trigger_matcher = crate::jobs::create_trigger_matcher(storage);
    Arc::new(
        crate::jobs::TriggerEvaluationHandler::new(job_registry, job_data_store, dispatcher)
            .with_trigger_matcher(trigger_matcher),
    )
}

/// Create the scheduled trigger handler
pub fn create_scheduled_trigger_handler(
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    dispatcher: Arc<JobDispatcher>,
    scheduled_trigger_finder: Option<ScheduledTriggerFinderCallback>,
) -> Arc<crate::jobs::ScheduledTriggerHandler> {
    let mut builder =
        crate::jobs::ScheduledTriggerHandler::new(job_registry, job_data_store, dispatcher);
    if let Some(finder) = scheduled_trigger_finder {
        builder = builder.with_trigger_finder(finder);
    }
    Arc::new(builder)
}

/// Create bulk SQL handler
pub fn create_bulk_sql_handler(
    sql_executor: Option<SqlExecutorCallback>,
) -> Arc<crate::jobs::BulkSqlHandler> {
    if let Some(executor) = sql_executor {
        Arc::new(crate::jobs::BulkSqlHandler::new().with_executor(executor))
    } else {
        Arc::new(crate::jobs::BulkSqlHandler::new())
    }
}

/// Create copy and restore tree handlers
pub fn create_tree_handlers(
    storage: &RocksDBStorage,
    copy_tree_executor: Option<crate::jobs::CopyTreeExecutorCallback>,
    restore_tree_executor: Option<crate::jobs::RestoreTreeExecutorCallback>,
) -> (
    Arc<crate::jobs::CopyTreeHandler>,
    Arc<crate::jobs::RestoreTreeHandler>,
    Arc<crate::jobs::RevisionHistoryCopyHandler>,
) {
    let revision_history_copy_handler = Arc::new(crate::jobs::RevisionHistoryCopyHandler::new(
        storage.db.clone(),
    ));

    let copy_tree_handler = if let Some(executor) = copy_tree_executor {
        Arc::new(crate::jobs::CopyTreeHandler::new().with_executor(executor))
    } else {
        Arc::new(crate::jobs::CopyTreeHandler::new())
    };

    let restore_tree_handler = if let Some(executor) = restore_tree_executor {
        Arc::new(crate::jobs::RestoreTreeHandler::new().with_executor(executor))
    } else {
        Arc::new(crate::jobs::RestoreTreeHandler::new())
    };

    (
        copy_tree_handler,
        restore_tree_handler,
        revision_history_copy_handler,
    )
}
