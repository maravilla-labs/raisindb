// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Job queuing callback for flow execution

use super::types::JobQueuerCallback;
use crate::execution::ExecutionDependencies;
use raisin_binary::BinaryStorage;
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::sync::Arc;

/// Create job queuer callback - queues background jobs
pub(super) fn create_job_queuer<S, B>(deps: &Arc<ExecutionDependencies<S, B>>) -> JobQueuerCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(
        move |job_type, payload, tenant_id, repo_id, branch, workspace| {
            let deps = deps.clone();
            Box::pin(async move {
                tracing::debug!(
                    job_type = %job_type,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    workspace = %workspace,
                    "Flow job_queuer callback"
                );

                // Get job registry from dependencies
                let job_registry = deps.job_registry.as_ref().ok_or_else(|| {
                    "Job registry not available in execution dependencies".to_string()
                })?;

                // Create a job type based on the job_type string
                use raisin_storage::jobs::JobType;

                let job = match job_type.as_str() {
                    "function_execution" => {
                        let function_path =
                            payload
                                .get("function_path")
                                .and_then(|v| v.as_str())
                                .ok_or_else(|| "Missing function_path in payload".to_string())?;

                        JobType::FunctionExecution {
                            function_path: function_path.to_string(),
                            trigger_name: Some("flow".to_string()),
                            execution_id: nanoid::nanoid!(),
                        }
                    }
                    "ai_call" => {
                        // AI call job for async AI Container execution
                        let instance_id = payload
                            .get("instance_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();

                        let step_id = payload
                            .get("step_id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| "Missing step_id in ai_call payload".to_string())?
                            .to_string();

                        let agent_ref = payload
                            .get("agent_ref")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| "Missing agent_ref in ai_call payload".to_string())?
                            .to_string();

                        let iteration = payload
                            .get("iteration")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u32;

                        JobType::AICall {
                            instance_id,
                            step_id,
                            agent_ref,
                            iteration,
                        }
                    }
                    _ => {
                        return Err(format!("Unsupported job type: {}", job_type));
                    }
                };

                // Register the job
                let job_id = job_registry
                    .register_job(job, Some(tenant_id.clone()), None, None, None)
                    .await
                    .map_err(|e| format!("Failed to register job: {}", e))?;

                // Store job context if we have a data store
                if let Some(data_store) = deps.job_data_store.as_ref() {
                    use raisin_hlc::HLC;
                    use raisin_storage::jobs::JobContext;

                    let mut metadata = std::collections::HashMap::new();

                    // For function execution, extract arguments as "input"
                    // (function_execution handler expects context.metadata["input"])
                    if job_type == "function_execution" {
                        if let Some(arguments) = payload.get("arguments") {
                            metadata.insert("input".to_string(), arguments.clone());
                        }
                        // Store step_id for flow resumption
                        if let Some(step_id) = payload.get("step_id") {
                            metadata.insert("step_id".to_string(), step_id.clone());
                        }
                        // Store instance_id for flow resumption after function completes
                        if let Some(instance_id) = payload.get("instance_id") {
                            metadata.insert("instance_id".to_string(), instance_id.clone());
                        }
                    }

                    // Store full payload for other job types or debugging
                    metadata.insert("payload".to_string(), payload);

                    let context = JobContext {
                        tenant_id: tenant_id.clone(),
                        repo_id: repo_id.clone(),
                        branch: branch.clone(),
                        workspace_id: workspace.clone(),
                        revision: HLC::new(0, 0),
                        metadata,
                    };

                    data_store
                        .put(&job_id, &context)
                        .map_err(|e| format!("Failed to store job context: {}", e))?;
                }

                Ok(job_id.to_string())
            })
        },
    )
}
