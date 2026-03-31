//! Trigger evaluation handler implementation.

use super::types::{
    NodeFetcherCallback, TriggerEvaluationReport, TriggerEvaluationResult, TriggerEventInfo,
    TriggerMatch, TriggerMatcherCallback,
};
use crate::jobs::data_store::JobDataStore;
use crate::jobs::dispatcher::JobDispatcher;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobRegistry, JobType};
use std::collections::HashMap;
use std::sync::Arc;

/// Handler for trigger evaluation jobs
///
/// This handler processes TriggerEvaluation jobs by finding matching triggers
/// and enqueueing FunctionExecution jobs for each match.
pub struct TriggerEvaluationHandler {
    /// Job registry for enqueueing function execution jobs
    job_registry: Arc<JobRegistry>,
    /// Job data store for storing job context
    job_data_store: Arc<JobDataStore>,
    /// Job dispatcher for routing jobs to worker queues
    dispatcher: Arc<JobDispatcher>,
    /// Optional trigger matcher callback (created by storage layer)
    trigger_matcher: Option<TriggerMatcherCallback>,
    /// Optional node fetcher callback (set by transport layer)
    node_fetcher: Option<NodeFetcherCallback>,
}

impl TriggerEvaluationHandler {
    /// Create a new trigger evaluation job handler
    pub fn new(
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        dispatcher: Arc<JobDispatcher>,
    ) -> Self {
        Self {
            job_registry,
            job_data_store,
            dispatcher,
            trigger_matcher: None,
            node_fetcher: None,
        }
    }

    /// Set the trigger matcher callback
    ///
    /// This should be called during job system initialization to provide
    /// the callback that finds matching triggers with detailed debug info.
    pub fn with_trigger_matcher(mut self, matcher: TriggerMatcherCallback) -> Self {
        self.trigger_matcher = Some(matcher);
        self
    }

    /// Set the node fetcher callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the callback that fetches node data for function context.
    pub fn with_node_fetcher(mut self, fetcher: NodeFetcherCallback) -> Self {
        self.node_fetcher = Some(fetcher);
        self
    }

    /// Handle trigger evaluation job
    ///
    /// Finds all matching triggers and enqueues FunctionExecution jobs for each.
    /// Returns a detailed TriggerEvaluationReport for debugging.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::TriggerEvaluation variant
    /// * `context` - Job context with tenant, repo, branch, workspace info
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        let start_time = std::time::Instant::now();

        // Extract event info from JobType
        let (event_type, node_id, node_type) = match &job.job_type {
            JobType::TriggerEvaluation {
                event_type,
                node_id,
                node_type,
            } => (event_type.clone(), node_id.clone(), node_type.clone()),
            _ => {
                return Err(Error::Validation(
                    "Expected TriggerEvaluation job type".to_string(),
                ))
            }
        };

        // Validate tenant/repo context is properly set for isolation
        if context.tenant_id.is_empty() {
            return Err(Error::Validation(
                "Trigger evaluation requires non-empty tenant_id for proper isolation".to_string(),
            ));
        }
        if context.repo_id.is_empty() {
            return Err(Error::Validation(
                "Trigger evaluation requires non-empty repo_id for proper isolation".to_string(),
            ));
        }

        tracing::info!(
            job_id = %job.id,
            event_type = %event_type,
            node_id = %node_id,
            node_type = %node_type,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace = %context.workspace_id,
            "Processing trigger evaluation job"
        );

        // Get node path from context metadata
        let node_path = context
            .metadata
            .get("node_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/")
            .to_string();

        // Get node properties from context metadata for reporting
        let node_properties = context.metadata.get("node_data").cloned();

        // Build the event info for the report
        let event_info = TriggerEventInfo {
            event_type: event_type.clone(),
            node_id: node_id.clone(),
            node_type: node_type.clone(),
            node_path: node_path.clone(),
            workspace: context.workspace_id.clone(),
            node_properties: node_properties.clone(),
        };

        // Find matching triggers using the matcher callback
        let (matches, mut trigger_results) = if let Some(matcher) = self.trigger_matcher.as_ref() {
            matcher(
                event_type.clone(),
                node_id.clone(),
                node_type.clone(),
                node_path.clone(),
                context.tenant_id.clone(),
                context.repo_id.clone(),
                context.branch.clone(),
                context.workspace_id.clone(),
                node_properties,
            )
            .await?
        } else {
            tracing::debug!(
                job_id = %job.id,
                event_type = %event_type,
                node_id = %node_id,
                "Trigger matcher not configured, skipping trigger evaluation"
            );
            let report = TriggerEvaluationReport {
                event: event_info,
                triggers_evaluated: 0,
                triggers_matched: 0,
                trigger_results: vec![],
                duration_ms: start_time.elapsed().as_millis() as u64,
            };
            return Ok(Some(serde_json::to_value(report).unwrap_or_default()));
        };

        if matches.is_empty() {
            tracing::debug!(
                job_id = %job.id,
                event_type = %event_type,
                node_type = %node_type,
                "No matching triggers found"
            );
            let report = TriggerEvaluationReport {
                event: event_info,
                triggers_evaluated: trigger_results.len(),
                triggers_matched: 0,
                trigger_results,
                duration_ms: start_time.elapsed().as_millis() as u64,
            };
            return Ok(Some(serde_json::to_value(report).unwrap_or_default()));
        }

        tracing::info!(
            job_id = %job.id,
            event_type = %event_type,
            node_id = %node_id,
            match_count = matches.len(),
            "Found matching triggers, enqueueing function executions"
        );

        // Sort by priority (higher first)
        let mut sorted_matches = matches;
        sorted_matches.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Track enqueued job IDs by trigger name
        let mut enqueued_jobs: HashMap<String, String> = HashMap::new();

        // Enqueue execution jobs for each match
        for trigger_match in sorted_matches {
            let enqueued_job_id = self
                .enqueue_trigger_match(
                    &trigger_match,
                    &event_type,
                    &node_id,
                    &node_type,
                    &node_path,
                    context,
                )
                .await?;

            if let Some(job_id) = enqueued_job_id {
                enqueued_jobs.insert(trigger_match.trigger_name.clone(), job_id);
            }
        }

        // Update trigger results with enqueued job IDs
        for result in &mut trigger_results {
            if result.matched {
                result.enqueued_job_id = enqueued_jobs.get(&result.trigger_name).cloned();
            }
        }

        let triggers_matched = trigger_results.iter().filter(|r| r.matched).count();

        let report = TriggerEvaluationReport {
            event: event_info,
            triggers_evaluated: trigger_results.len(),
            triggers_matched,
            trigger_results,
            duration_ms: start_time.elapsed().as_millis() as u64,
        };

        Ok(Some(serde_json::to_value(report).unwrap_or_default()))
    }

    /// Enqueue a single trigger match as a job.
    ///
    /// Returns the enqueued job ID if successful, or None if the trigger was skipped.
    async fn enqueue_trigger_match(
        &self,
        trigger_match: &TriggerMatch,
        event_type: &str,
        node_id: &str,
        node_type: &str,
        node_path: &str,
        context: &JobContext,
    ) -> Result<Option<String>> {
        let execution_id = nanoid::nanoid!();

        // Build execution context with event data
        let mut metadata = HashMap::new();
        metadata.insert(
            "trigger_name".to_string(),
            serde_json::json!(trigger_match.trigger_name),
        );
        metadata.insert("event_type".to_string(), serde_json::json!(event_type));
        metadata.insert("node_id".to_string(), serde_json::json!(node_id));
        metadata.insert("node_type".to_string(), serde_json::json!(node_type));
        metadata.insert("node_path".to_string(), serde_json::json!(node_path));

        // Build input with event data and workspace
        let flow_input = if let Some(node_data) = context.metadata.get("node_data") {
            serde_json::json!({
                "event": {
                    "type": event_type,
                    "node_id": node_id,
                    "node_type": node_type,
                    "node_path": node_path,
                },
                "node": node_data,
                "workspace": context.workspace_id,
            })
        } else {
            serde_json::json!({
                "event": {
                    "type": event_type,
                    "node_id": node_id,
                    "node_type": node_type,
                    "node_path": node_path,
                },
                "workspace": context.workspace_id,
            })
        };

        // Wrap in the same structure as FlowExecution for consistency
        let input_value = serde_json::json!({
            "flow_input": flow_input,
            "previous_results": {}
        });
        metadata.insert("input".to_string(), input_value);

        let job_context = JobContext {
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            branch: context.branch.clone(),
            workspace_id: context.workspace_id.clone(),
            revision: context.revision,
            metadata,
        };

        // Check for workflow_data (from referenced raisin:Flow node)
        if let Some(ref workflow_data) = trigger_match.workflow_data {
            let is_empty = workflow_data.is_null()
                || workflow_data
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(false);

            if !is_empty {
                return self
                    .enqueue_flow_instance(
                        trigger_match,
                        &flow_input,
                        event_type,
                        node_id,
                        node_type,
                        node_path,
                        workflow_data,
                        context,
                        &job_context,
                    )
                    .await;
            }
        }

        // Fall through to function_path for single-function triggers
        if let Some(ref function_path) = trigger_match.function_path {
            return self
                .enqueue_function_execution(
                    trigger_match,
                    function_path,
                    &execution_id,
                    &job_context,
                    context,
                )
                .await;
        }

        tracing::warn!(
            trigger_name = %trigger_match.trigger_name,
            "Trigger match has neither function_path nor workflow_data, skipping"
        );
        Ok(None)
    }

    /// Enqueue a flow instance execution job for a workflow trigger.
    #[allow(clippy::too_many_arguments)]
    async fn enqueue_flow_instance(
        &self,
        trigger_match: &TriggerMatch,
        flow_input: &serde_json::Value,
        event_type: &str,
        node_id: &str,
        node_type: &str,
        node_path: &str,
        workflow_data: &serde_json::Value,
        context: &JobContext,
        job_context: &JobContext,
    ) -> Result<Option<String>> {
        use chrono::Utc;
        use raisin_flow_runtime::integration::triggers::{FlowInstanceBuilder, FlowTriggerEvent};

        let trigger_path = trigger_match
            .trigger_path
            .clone()
            .unwrap_or_else(|| format!("/_triggers/{}", trigger_match.trigger_name));

        // Build the trigger event
        let trigger_event = FlowTriggerEvent::NodeEvent {
            event_type: event_type.to_string(),
            node_id: node_id.to_string(),
            node_type: node_type.to_string(),
            node_path: node_path.to_string(),
            properties: flow_input.clone(),
            timestamp: Utc::now(),
        };

        // Build the flow instance
        let instance = FlowInstanceBuilder::new(
            trigger_path.clone(),
            1, // version
            workflow_data.clone(),
            trigger_event,
            flow_input.clone(),
        )
        .tenant_id(context.tenant_id.clone())
        .repo_id(context.repo_id.clone())
        .branch(context.branch.clone())
        .workspace(context.workspace_id.clone())
        .build()
        .map_err(|e| Error::Backend(format!("Failed to create flow instance: {}", e)))?;

        let instance_id = instance.id.clone();

        // Queue FlowInstanceExecution job
        let flow_instance_job = JobType::FlowInstanceExecution {
            instance_id: instance_id.clone(),
            execution_type: "start".to_string(),
            resume_reason: None,
        };

        // Include the instance data in metadata for the handler to use
        let mut instance_metadata = job_context.metadata.clone();
        instance_metadata.insert(
            "flow_instance".to_string(),
            serde_json::to_value(&instance).unwrap_or(serde_json::Value::Null),
        );

        let instance_context = JobContext {
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            branch: context.branch.clone(),
            workspace_id: context.workspace_id.clone(),
            revision: context.revision,
            metadata: instance_metadata,
        };

        let job_id = self
            .job_registry
            .register_job(
                flow_instance_job.clone(),
                Some(context.tenant_id.clone()),
                None,
                None,
                trigger_match.max_retries,
            )
            .await?;

        self.job_data_store.put(&job_id, &instance_context)?;

        // Dispatch to priority queue (non-blocking to prevent upload stalls)
        let priority = flow_instance_job.default_priority();
        if !self.dispatcher.try_dispatch(job_id.clone(), priority) {
            tracing::warn!(
                job_id = %job_id,
                priority = %priority,
                "Queue full, flow instance job not dispatched - will be picked up on next poll"
            );
        }

        tracing::debug!(
            job_id = %job_id,
            instance_id = %instance_id,
            trigger_path = %trigger_path,
            trigger_name = %trigger_match.trigger_name,
            priority = %priority,
            "Enqueued and dispatched flow instance execution job"
        );

        Ok(Some(job_id.to_string()))
    }

    /// Enqueue a function execution job for a single-function trigger.
    async fn enqueue_function_execution(
        &self,
        trigger_match: &TriggerMatch,
        function_path: &str,
        execution_id: &str,
        job_context: &JobContext,
        context: &JobContext,
    ) -> Result<Option<String>> {
        let function_job_type = JobType::FunctionExecution {
            function_path: function_path.to_string(),
            trigger_name: Some(trigger_match.trigger_name.clone()),
            execution_id: execution_id.to_string(),
        };

        let job_id = self
            .job_registry
            .register_job(
                function_job_type.clone(),
                Some(context.tenant_id.clone()),
                None,
                None,
                trigger_match.max_retries,
            )
            .await?;

        self.job_data_store.put(&job_id, job_context)?;

        // Dispatch to priority queue (non-blocking to prevent upload stalls)
        let priority = function_job_type.default_priority();
        if !self.dispatcher.try_dispatch(job_id.clone(), priority) {
            tracing::warn!(
                job_id = %job_id,
                priority = %priority,
                "Queue full, function execution job not dispatched - will be picked up on next poll"
            );
        }

        tracing::debug!(
            job_id = %job_id,
            execution_id = %execution_id,
            function_path = %function_path,
            trigger_name = %trigger_match.trigger_name,
            priority = %priority,
            "Enqueued and dispatched function execution job"
        );

        Ok(Some(job_id.to_string()))
    }
}
