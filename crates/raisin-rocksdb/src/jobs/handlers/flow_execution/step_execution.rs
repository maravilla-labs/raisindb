// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step execution logic for flow handler
//!
//! Contains the individual step execution, parallel function dispatch,
//! and sequential function execution methods.

use raisin_error::Result;
use raisin_storage::jobs::JobContext;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use super::types::*;
use super::{FlowExecutionHandler, FunctionExecutorCallback, FUNCTIONS_WORKSPACE};

impl FlowExecutionHandler {
    /// Execute a single step (may run multiple functions)
    pub(super) async fn execute_step(
        &self,
        step: &FlowStep,
        flow_execution_id: &str,
        flow_input: &serde_json::Value,
        previous_results: &HashMap<String, StepResult>,
        executor: &FunctionExecutorCallback,
        context: &JobContext,
    ) -> Result<StepResult> {
        let step_start = Instant::now();

        tracing::debug!(
            flow_execution_id = %flow_execution_id,
            step_id = %step.id,
            step_name = %step.name,
            parallel = step.parallel,
            function_count = step.functions.len(),
            "Executing step"
        );

        // Build input for this step (combine flow input with previous results)
        let step_input = serde_json::json!({
            "flow_input": flow_input,
            "previous_results": previous_results,
        });

        let function_results = if step.parallel && step.functions.len() > 1 {
            // Execute functions in parallel
            self.execute_parallel(step, flow_execution_id, &step_input, executor, context)
                .await?
        } else {
            // Execute functions sequentially
            self.execute_sequential(step, flow_execution_id, &step_input, executor, context)
                .await?
        };

        let duration_ms = step_start.elapsed().as_millis() as u64;

        // Determine step status based on function results and error behavior
        let any_failed = function_results.iter().any(|r| !r.success);
        let all_failed = function_results.iter().all(|r| !r.success);

        let status = if all_failed {
            StepStatus::Failed
        } else if any_failed {
            match step.on_error {
                StepErrorBehavior::Stop => StepStatus::Failed,
                StepErrorBehavior::Skip | StepErrorBehavior::Continue => StepStatus::Completed,
            }
        } else {
            StepStatus::Completed
        };

        let error = if any_failed {
            let errors: Vec<String> = function_results
                .iter()
                .filter(|r| !r.success)
                .filter_map(|r| {
                    r.error
                        .as_ref()
                        .map(|e| format!("{}: {}", r.function_path, e))
                })
                .collect();
            if errors.is_empty() {
                None
            } else {
                Some(errors.join("; "))
            }
        } else {
            None
        };

        Ok(StepResult {
            step_id: step.id.clone(),
            status,
            function_results,
            duration_ms,
            error,
        })
    }

    /// Execute functions in parallel using tokio::join_all
    async fn execute_parallel(
        &self,
        step: &FlowStep,
        flow_execution_id: &str,
        step_input: &serde_json::Value,
        executor: &FunctionExecutorCallback,
        context: &JobContext,
    ) -> Result<Vec<FunctionResult>> {
        let futures: Vec<_> = step
            .functions
            .iter()
            .map(|func| {
                let executor = Arc::clone(executor);
                let execution_id = format!("{}-{}", flow_execution_id, nanoid::nanoid!(8));
                let function_path = func.path.clone();
                let tenant_id = context.tenant_id.clone();
                let repo_id = context.repo_id.clone();
                let branch = context.branch.clone();
                let workspace = FUNCTIONS_WORKSPACE.to_string();
                let input = step_input.clone();

                async move {
                    let start = Instant::now();
                    let result = executor(
                        function_path.clone(),
                        execution_id.clone(),
                        input,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        None, // System context for trigger-invoked flows
                        None, // No real-time log streaming for parallel flow steps
                    )
                    .await;

                    match result {
                        Ok(exec_result) => FunctionResult {
                            function_path,
                            execution_id,
                            success: exec_result.success,
                            result: exec_result.result,
                            error: exec_result.error,
                            duration_ms: exec_result.duration_ms,
                        },
                        Err(e) => FunctionResult {
                            function_path,
                            execution_id,
                            success: false,
                            result: None,
                            error: Some(e.to_string()),
                            duration_ms: start.elapsed().as_millis() as u64,
                        },
                    }
                }
            })
            .collect();

        Ok(futures::future::join_all(futures).await)
    }

    /// Execute functions sequentially
    async fn execute_sequential(
        &self,
        step: &FlowStep,
        flow_execution_id: &str,
        step_input: &serde_json::Value,
        executor: &FunctionExecutorCallback,
        context: &JobContext,
    ) -> Result<Vec<FunctionResult>> {
        let mut results = Vec::new();

        for func in &step.functions {
            let execution_id = format!("{}-{}", flow_execution_id, nanoid::nanoid!(8));
            let start = Instant::now();

            let result = executor(
                func.path.clone(),
                execution_id.clone(),
                step_input.clone(),
                context.tenant_id.clone(),
                context.repo_id.clone(),
                context.branch.clone(),
                FUNCTIONS_WORKSPACE.to_string(),
                None, // System context for trigger-invoked flows
                None, // No real-time log streaming for sequential flow steps
            )
            .await;

            let function_result = match result {
                Ok(exec_result) => FunctionResult {
                    function_path: func.path.clone(),
                    execution_id,
                    success: exec_result.success,
                    result: exec_result.result,
                    error: exec_result.error,
                    duration_ms: exec_result.duration_ms,
                },
                Err(e) => FunctionResult {
                    function_path: func.path.clone(),
                    execution_id,
                    success: false,
                    result: None,
                    error: Some(e.to_string()),
                    duration_ms: start.elapsed().as_millis() as u64,
                },
            };

            let failed = !function_result.success;
            results.push(function_result);

            // Stop on failure if step error behavior is Stop
            if failed && step.on_error == StepErrorBehavior::Stop {
                break;
            }
        }

        Ok(results)
    }
}
