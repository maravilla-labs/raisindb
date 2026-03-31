// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Main flow execution handler logic
//!
//! Orchestrates the execution of a multi-function flow by processing steps
//! in topological order, handling error strategies, and aggregating results.

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::collections::HashMap;
use std::time::Instant;

use super::types::*;
use super::FlowExecutionHandler;

impl FlowExecutionHandler {
    /// Handle flow execution job
    ///
    /// Orchestrates the execution of a multi-function flow, handling
    /// sequential/parallel execution, error handling, and result aggregation.
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract flow info from JobType
        let (flow_execution_id, trigger_path, flow_value, current_step_index, step_results_value) =
            match &job.job_type {
                JobType::FlowExecution {
                    flow_execution_id,
                    trigger_path,
                    flow,
                    current_step_index,
                    step_results,
                } => (
                    flow_execution_id.clone(),
                    trigger_path.clone(),
                    flow.clone(),
                    *current_step_index,
                    step_results.clone(),
                ),
                _ => {
                    return Err(Error::Validation(
                        "Expected FlowExecution job type".to_string(),
                    ))
                }
            };

        tracing::info!(
            job_id = %job.id,
            flow_execution_id = %flow_execution_id,
            trigger_path = %trigger_path,
            current_step_index = current_step_index,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace = %context.workspace_id,
            "Processing flow execution job"
        );

        // Check if executor is available
        let executor = self.executor.as_ref().ok_or_else(|| {
            Error::Validation(
                "Function executor not configured. The transport layer must provide the executor callback.".to_string()
            )
        })?;

        // Parse the flow definition
        let flow: FunctionFlow = serde_json::from_value(flow_value.clone())
            .map_err(|e| Error::Validation(format!("Failed to parse function flow: {}", e)))?;

        // Parse existing step results
        let mut step_results: HashMap<String, StepResult> = if step_results_value.is_null()
            || step_results_value.is_object()
                && step_results_value
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(true)
        {
            HashMap::new()
        } else {
            serde_json::from_value(step_results_value.clone())
                .map_err(|e| Error::Validation(format!("Failed to parse step results: {}", e)))?
        };

        // Get execution order
        let execution_order = flow
            .execution_order()
            .map_err(|e| Error::Validation(format!("Invalid flow: {}", e)))?;

        let flow_start = Instant::now();
        let mut final_error: Option<String> = None;
        let mut overall_status = FlowStatus::Running;

        // Get input from job context metadata
        let flow_input = context
            .metadata
            .get("input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // Execute steps in order
        for step in execution_order.iter().skip(current_step_index) {
            // Check if dependencies are satisfied
            let deps_satisfied = step.depends_on.iter().all(|dep_id| {
                step_results
                    .get(dep_id)
                    .map(|r| r.status == StepStatus::Completed)
                    .unwrap_or(false)
            });

            if !deps_satisfied {
                // Check if any dependency failed
                let any_dep_failed = step.depends_on.iter().any(|dep_id| {
                    step_results
                        .get(dep_id)
                        .map(|r| r.status == StepStatus::Failed)
                        .unwrap_or(false)
                });

                if any_dep_failed && flow.error_strategy == ErrorStrategy::FailFast {
                    tracing::info!(
                        flow_execution_id = %flow_execution_id,
                        step_id = %step.id,
                        "Skipping step due to failed dependency (fail_fast strategy)"
                    );

                    step_results.insert(
                        step.id.clone(),
                        StepResult {
                            step_id: step.id.clone(),
                            status: StepStatus::Skipped,
                            function_results: vec![],
                            duration_ms: 0,
                            error: Some("Skipped due to dependency failure".to_string()),
                        },
                    );
                    continue;
                }
            }

            // Execute the step
            let step_result = self
                .execute_step(
                    step,
                    &flow_execution_id,
                    &flow_input,
                    &step_results,
                    executor,
                    context,
                )
                .await;

            match step_result {
                Ok(result) => {
                    let step_failed = result.status == StepStatus::Failed;
                    let error_detail = result
                        .error
                        .as_ref()
                        .map(|e| format!(": {}", e))
                        .unwrap_or_default();
                    step_results.insert(step.id.clone(), result);

                    if step_failed && flow.error_strategy == ErrorStrategy::FailFast {
                        final_error = Some(format!("Step '{}' failed{}", step.id, error_detail));
                        overall_status = FlowStatus::Failed;
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!(
                        flow_execution_id = %flow_execution_id,
                        step_id = %step.id,
                        error = %e,
                        "Step execution failed with error"
                    );

                    step_results.insert(
                        step.id.clone(),
                        StepResult {
                            step_id: step.id.clone(),
                            status: StepStatus::Failed,
                            function_results: vec![],
                            duration_ms: 0,
                            error: Some(e.to_string()),
                        },
                    );

                    if flow.error_strategy == ErrorStrategy::FailFast {
                        final_error = Some(e.to_string());
                        overall_status = FlowStatus::Failed;
                        break;
                    }
                }
            }
        }

        // Determine final status
        if overall_status == FlowStatus::Running {
            let any_failed = step_results
                .values()
                .any(|r| r.status == StepStatus::Failed);
            overall_status = if any_failed {
                if flow.error_strategy == ErrorStrategy::Continue {
                    FlowStatus::PartialSuccess
                } else {
                    FlowStatus::Failed
                }
            } else {
                FlowStatus::Completed
            };
        }

        let duration_ms = flow_start.elapsed().as_millis() as u64;

        // Build result
        let result = FlowExecutionResult {
            flow_execution_id: flow_execution_id.clone(),
            trigger_path,
            status: overall_status,
            started_at: chrono::Utc::now() - chrono::Duration::milliseconds(duration_ms as i64),
            completed_at: Some(chrono::Utc::now()),
            duration_ms,
            step_results: step_results.into_values().collect(),
            final_output: None, // Could aggregate outputs from final steps
            error: final_error.clone(),
        };

        tracing::info!(
            flow_execution_id = %flow_execution_id,
            status = ?overall_status,
            duration_ms = duration_ms,
            step_count = result.step_results.len(),
            "Flow execution completed"
        );

        // Return error if flow failed
        if overall_status == FlowStatus::Failed {
            if let Some(error) = final_error {
                return Err(Error::Backend(format!("Flow execution failed: {}", error)));
            }
        }

        // Return the result as JSON
        let result_json = serde_json::to_value(&result)
            .map_err(|e| Error::Backend(format!("Failed to serialize flow result: {}", e)))?;

        Ok(Some(result_json))
    }
}
