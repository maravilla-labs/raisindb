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

                // Child-flow job types create a new FlowInstance and return
                // the child INSTANCE id (callers track children by it).
                match job_type.as_str() {
                    // Sub-flow step: child flow referenced by path
                    "flow_execution" => {
                        let flow_path = payload
                            .get("flow_path")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "Missing flow_path in flow_execution payload".to_string()
                            })?
                            .to_string();
                        let parent_instance_id = payload
                            .get("parent_instance_id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "Missing parent_instance_id in flow_execution payload".to_string()
                            })?
                            .to_string();
                        let parent_step_id = payload
                            .get("parent_step_id")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let input = payload
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);

                        let (workflow_data, flow_version) =
                            load_flow_definition(&deps, &tenant_id, &repo_id, &branch, &flow_path)
                                .await?;

                        return create_child_flow_instance(
                            &deps,
                            &tenant_id,
                            &repo_id,
                            &branch,
                            flow_path,
                            flow_version,
                            workflow_data,
                            input,
                            parent_instance_id,
                            parent_step_id,
                            None,
                        )
                        .await;
                    }
                    // Parallel branch: child flow with an inline definition
                    "create_child_flow" => {
                        let parent_instance_id = payload
                            .get("parent_instance_id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "Missing parent_instance_id in create_child_flow payload"
                                    .to_string()
                            })?
                            .to_string();
                        let branch_id = payload
                            .get("branch_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("branch")
                            .to_string();
                        let flow_definition =
                            payload.get("flow_definition").cloned().ok_or_else(|| {
                                "Missing flow_definition in create_child_flow payload".to_string()
                            })?;
                        let input = payload
                            .get("input")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);

                        let flow_ref = format!("inline:{}#{}", parent_instance_id, branch_id);

                        return create_child_flow_instance(
                            &deps,
                            &tenant_id,
                            &repo_id,
                            &branch,
                            flow_ref,
                            1,
                            flow_definition,
                            input,
                            parent_instance_id,
                            payload
                                .get("parent_step_id")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            Some(branch_id),
                        )
                        .await;
                    }
                    _ => {}
                }

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
                    "FlowInstanceExecution" => {
                        // Flow lifecycle jobs queued by the flow runtime itself
                        // (timeout checks, scheduled-wait wakeups, retry wakeups).
                        let instance_id = payload
                            .get("instance_id")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                "Missing instance_id in FlowInstanceExecution payload".to_string()
                            })?
                            .to_string();

                        let execution_type = payload
                            .get("execution_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("timeout_check")
                            .to_string();

                        let resume_reason = payload
                            .get("resume_reason")
                            .and_then(|v| v.as_str())
                            .map(String::from);

                        JobType::FlowInstanceExecution {
                            instance_id,
                            execution_type,
                            resume_reason,
                        }
                    }
                    _ => {
                        return Err(format!("Unsupported job type: {}", job_type));
                    }
                };

                // Honor a future schedule if requested (see
                // FlowCallbacks::queue_job_at): dispatch at the given time
                // instead of immediately.
                let scheduled_at = payload
                    .get("__scheduled_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                // Register the job
                let job_id = match scheduled_at {
                    Some(at) if at > chrono::Utc::now() => job_registry
                        .register_job_at(job, tenant_id.clone(), at, None)
                        .await
                        .map_err(|e| format!("Failed to register scheduled job: {}", e))?,
                    _ => job_registry
                        .register_job(job, tenant_id.clone(), None, None, None)
                        .await
                        .map_err(|e| format!("Failed to register job: {}", e))?,
                };

                // Store job context if we have a data store
                if let Some(data_store) = deps.job_data_store.as_ref() {
                    use raisin_hlc::HLC;
                    use raisin_storage::jobs::JobContext;

                    let mut metadata = std::collections::HashMap::new();

                    // For flow resume jobs, surface resume_data where the
                    // FlowInstanceExecution handler expects it
                    // (context.metadata["function_result"])
                    if job_type == "FlowInstanceExecution" {
                        if let Some(resume_data) = payload.get("resume_data") {
                            metadata.insert("function_result".to_string(), resume_data.clone());
                        }
                    }

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

/// Load a `raisin:Flow` node's workflow_data + version for sub-flow creation.
async fn load_flow_definition<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    flow_path: &str,
) -> Result<(serde_json::Value, i32), String>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_storage::{NodeRepository, StorageScope};

    let scope = StorageScope::new(tenant_id, repo_id, branch, "functions");
    let flow_node = deps
        .storage
        .nodes()
        .get_by_path(scope, flow_path, None)
        .await
        .map_err(|e| format!("Failed to load flow '{}': {}", flow_path, e))?
        .ok_or_else(|| format!("Flow '{}' not found", flow_path))?;

    if flow_node.node_type != "raisin:Flow" {
        return Err(format!(
            "Node '{}' is not a raisin:Flow (found: {})",
            flow_path, flow_node.node_type
        ));
    }

    let workflow_data = flow_node
        .properties
        .get("workflow_data")
        .and_then(|v| serde_json::to_value(v).ok())
        .ok_or_else(|| format!("Flow '{}' missing workflow_data property", flow_path))?;

    let flow_version = flow_node
        .properties
        .get("version")
        .and_then(|v| match v {
            PropertyValue::Integer(i) => Some(*i as i32),
            _ => None,
        })
        .unwrap_or(1);

    Ok((workflow_data, flow_version))
}

/// Create a child flow instance (sub-flow or parallel branch) and queue its
/// start job. Returns the child INSTANCE id.
///
/// The parent linkage (`parent_instance_ref` + `__parent_step_id` /
/// `__branch_id` variables) is what makes the child's terminal state resume
/// the parent (see `notify_parent_flow` in raisin-flow-runtime).
#[allow(clippy::too_many_arguments)]
async fn create_child_flow_instance<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    flow_ref: String,
    flow_version: i32,
    workflow_data: serde_json::Value,
    input: serde_json::Value,
    parent_instance_id: String,
    parent_step_id: Option<String>,
    branch_id: Option<String>,
) -> Result<String, String>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    use raisin_flow_runtime::integration::{FlowInstanceBuilder, FlowTriggerEvent};
    use raisin_storage::jobs::{JobContext, JobType};

    let trigger = FlowTriggerEvent::Manual {
        actor: format!("flow:{}", parent_instance_id),
        actor_home: None,
        timestamp: chrono::Utc::now(),
    };

    let mut instance = FlowInstanceBuilder::new(
        flow_ref.clone(),
        flow_version,
        workflow_data,
        trigger,
        input,
    )
    .tenant_id(tenant_id.to_string())
    .repo_id(repo_id.to_string())
    .branch(branch.to_string())
    .workspace("functions".to_string())
    .build()
    .map_err(|e| format!("Failed to build child flow instance: {}", e))?;

    instance.parent_instance_ref = Some(parent_instance_id.clone());
    if let serde_json::Value::Object(ref mut vars) = instance.variables {
        if let Some(step_id) = &parent_step_id {
            vars.insert(
                "__parent_step_id".to_string(),
                serde_json::Value::String(step_id.clone()),
            );
        }
        if let Some(bid) = &branch_id {
            vars.insert(
                "__branch_id".to_string(),
                serde_json::Value::String(bid.clone()),
            );
        }
    }

    let job_registry = deps
        .job_registry
        .as_ref()
        .ok_or_else(|| "Job registry not available in execution dependencies".to_string())?;

    let instance_id = instance.id.clone();

    let job = JobType::FlowInstanceExecution {
        instance_id: instance_id.clone(),
        execution_type: "start".to_string(),
        resume_reason: Some("child_flow".to_string()),
    };

    let job_id = job_registry
        .register_job(job, tenant_id.to_string(), None, None, None)
        .await
        .map_err(|e| format!("Failed to register child flow job: {}", e))?;

    if let Some(data_store) = deps.job_data_store.as_ref() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "flow_instance".to_string(),
            serde_json::to_value(&instance)
                .map_err(|e| format!("Failed to serialize child instance: {}", e))?,
        );

        let context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace_id: "raisin:system".to_string(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        data_store
            .put(&job_id, &context)
            .map_err(|e| format!("Failed to store child flow job context: {}", e))?;
    }

    tracing::info!(
        child_instance_id = %instance_id,
        parent_instance_id = %parent_instance_id,
        flow_ref = %flow_ref,
        job_id = %job_id,
        "Created child flow instance and queued start job"
    );

    Ok(instance_id)
}
