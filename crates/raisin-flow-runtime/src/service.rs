// SPDX-License-Identifier: BSL-1.1

//! Transport-agnostic flow service.
//!
//! Encapsulates core flow operations (run, resume, get status, cancel) so that
//! both the HTTP and WebSocket transport layers delegate here instead of
//! duplicating the logic.

use std::collections::HashMap;

use async_trait::async_trait;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::JobType;
use raisin_storage::{NodeRepository, Storage, StorageScope, UpdateNodeOptions};
use serde::Serialize;
use serde_json::Value;

use crate::integration::triggers::{FlowInstanceBuilder, FlowTriggerEvent};
use crate::types::{FlowError, FlowInstance, FlowStatus};

const TENANT_ID: &str = "default";
const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";
const SYSTEM_WORKSPACE: &str = "raisin:system";

// ---------------------------------------------------------------------------
// Job scheduling trait
// ---------------------------------------------------------------------------

/// Abstraction for scheduling flow jobs.
///
/// Implemented by storage backends that support background job execution
/// (e.g., RocksDB). Transport handlers pass an implementation of this trait
/// to the service functions.
#[async_trait]
pub trait FlowJobScheduler: Send + Sync {
    /// Register a flow job and store its execution context.
    ///
    /// Returns the job ID on success.
    async fn schedule_flow_job(
        &self,
        repo: &str,
        job_type: JobType,
        metadata: HashMap<String, Value>,
    ) -> Result<String, FlowError>;

    /// Best-effort cancel any jobs associated with the given flow instance.
    async fn cancel_flow_jobs(&self, instance_id: &str) -> Result<(), FlowError>;
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of starting or resuming a flow.
#[derive(Debug, Clone, Serialize)]
pub struct FlowRunResult {
    pub instance_id: String,
    pub job_id: String,
}

/// Current status of a flow instance.
#[derive(Debug, Clone, Serialize)]
pub struct FlowInstanceStatus {
    pub id: String,
    pub status: String,
    pub variables: Value,
    pub flow_path: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Validated data extracted from a `raisin:Flow` node.
struct ValidatedFlow {
    workflow_data: Value,
    flow_version: i32,
}

/// Load a `raisin:Flow` node by path and validate it.
async fn load_and_validate_flow<S: Storage>(
    storage: &S,
    repo: &str,
    flow_path: &str,
) -> Result<ValidatedFlow, FlowError> {
    let scope = StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE);
    let flow_node = storage
        .nodes()
        .get_by_path(scope, flow_path, None)
        .await
        .map_err(|e| FlowError::Other(e.to_string()))?
        .ok_or_else(|| FlowError::NodeNotFound(format!("Flow '{}' not found", flow_path)))?;

    if flow_node.node_type != "raisin:Flow" {
        return Err(FlowError::InvalidDefinition(format!(
            "Node '{}' is not a raisin:Flow (found: {})",
            flow_path, flow_node.node_type
        )));
    }

    let workflow_data = flow_node
        .properties
        .get("workflow_data")
        .and_then(|v| serde_json::to_value(v).ok())
        .ok_or_else(|| {
            FlowError::InvalidDefinition("Flow node missing workflow_data property".to_string())
        })?;

    let is_empty = workflow_data.is_null()
        || workflow_data
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(false);
    if is_empty {
        return Err(FlowError::InvalidDefinition(
            "Flow workflow_data is empty - please define the flow steps".to_string(),
        ));
    }

    let flow_version = flow_node
        .properties
        .get("version")
        .and_then(|v| match v {
            PropertyValue::Integer(i) => Some(*i as i32),
            _ => None,
        })
        .unwrap_or(1);

    Ok(ValidatedFlow {
        workflow_data,
        flow_version,
    })
}

/// Load a flow instance node and deserialize it.
async fn load_instance<S: Storage>(
    storage: &S,
    repo: &str,
    instance_id: &str,
) -> Result<(raisin_models::nodes::Node, FlowInstance), FlowError> {
    let scope = StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, SYSTEM_WORKSPACE);
    let instance_path = format!("/flows/instances/{}", instance_id);

    let instance_node = storage
        .nodes()
        .get_by_path(scope, &instance_path, None)
        .await
        .map_err(|e| FlowError::Other(e.to_string()))?
        .ok_or_else(|| {
            FlowError::NodeNotFound(format!("Flow instance '{}' not found", instance_id))
        })?;

    let instance: FlowInstance = serde_json::from_value(
        serde_json::to_value(&instance_node.properties)
            .map_err(|e| FlowError::Serialization(e.to_string()))?,
    )
    .map_err(|e| FlowError::Serialization(e.to_string()))?;

    Ok((instance_node, instance))
}

// ---------------------------------------------------------------------------
// Public service functions
// ---------------------------------------------------------------------------

/// Start a new flow execution.
///
/// Validates the flow node, creates a `FlowInstance`, and queues a job.
pub async fn run_flow<S: Storage>(
    storage: &S,
    scheduler: &dyn FlowJobScheduler,
    repo: &str,
    flow_path: &str,
    input: Value,
    actor: String,
    actor_home: Option<String>,
) -> Result<FlowRunResult, FlowError> {
    let flow = load_and_validate_flow(storage, repo, flow_path).await?;

    let trigger = FlowTriggerEvent::Manual {
        actor,
        actor_home,
        timestamp: chrono::Utc::now(),
    };

    let instance = FlowInstanceBuilder::new(
        flow_path.to_string(),
        flow.flow_version,
        flow.workflow_data,
        trigger,
        input,
    )
    .tenant_id(TENANT_ID.to_string())
    .repo_id(repo.to_string())
    .branch(DEFAULT_BRANCH.to_string())
    .workspace(FUNCTIONS_WORKSPACE.to_string())
    .build()
    .map_err(|e| FlowError::Other(format!("Failed to create flow instance: {}", e)))?;

    let instance_id = instance.id.clone();

    let job_type = JobType::FlowInstanceExecution {
        instance_id: instance_id.clone(),
        execution_type: "start".to_string(),
        resume_reason: None,
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "flow_instance".to_string(),
        serde_json::to_value(&instance).unwrap_or(Value::Null),
    );

    let job_id = scheduler
        .schedule_flow_job(repo, job_type, metadata)
        .await?;

    tracing::info!(
        job_id = %job_id,
        instance_id = %instance_id,
        flow_path = %flow_path,
        "Queued flow instance execution"
    );

    Ok(FlowRunResult {
        instance_id,
        job_id,
    })
}

/// Start a new flow execution in test mode.
pub async fn run_flow_test<S: Storage>(
    storage: &S,
    scheduler: &dyn FlowJobScheduler,
    repo: &str,
    flow_path: &str,
    input: Value,
    mut test_config: crate::types::TestRunConfig,
) -> Result<FlowRunResult, FlowError> {
    let flow = load_and_validate_flow(storage, repo, flow_path).await?;

    let trigger = FlowTriggerEvent::Manual {
        actor: "test_api".to_string(),
        actor_home: None,
        timestamp: chrono::Utc::now(),
    };

    test_config.is_test_run = true;

    let instance = FlowInstanceBuilder::new(
        flow_path.to_string(),
        flow.flow_version,
        flow.workflow_data,
        trigger,
        input,
    )
    .tenant_id(TENANT_ID.to_string())
    .repo_id(repo.to_string())
    .branch(DEFAULT_BRANCH.to_string())
    .workspace(FUNCTIONS_WORKSPACE.to_string())
    .test_config(test_config)
    .build()
    .map_err(|e| FlowError::Other(format!("Failed to create flow instance: {}", e)))?;

    let instance_id = instance.id.clone();

    let job_type = JobType::FlowInstanceExecution {
        instance_id: instance_id.clone(),
        execution_type: "test".to_string(),
        resume_reason: None,
    };

    let mut metadata = HashMap::new();
    metadata.insert(
        "flow_instance".to_string(),
        serde_json::to_value(&instance).unwrap_or(Value::Null),
    );
    metadata.insert("is_test_run".to_string(), serde_json::json!(true));

    let job_id = scheduler
        .schedule_flow_job(repo, job_type, metadata)
        .await?;

    tracing::info!(
        job_id = %job_id,
        instance_id = %instance_id,
        flow_path = %flow_path,
        is_test_run = true,
        "Queued test flow instance execution"
    );

    Ok(FlowRunResult {
        instance_id,
        job_id,
    })
}

/// Resume a paused flow instance.
///
/// Verifies the instance is in `Waiting` state and queues a resume job.
pub async fn resume_flow<S: Storage>(
    storage: &S,
    scheduler: &dyn FlowJobScheduler,
    repo: &str,
    instance_id: &str,
    resume_data: Value,
) -> Result<FlowRunResult, FlowError> {
    let (_node, instance) = load_instance(storage, repo, instance_id).await?;

    if instance.status != FlowStatus::Waiting {
        return Err(FlowError::InvalidStateTransition {
            from: instance.status.as_str().to_string(),
            to: "resumed".to_string(),
        });
    }

    let job_type = JobType::FlowInstanceExecution {
        instance_id: instance_id.to_string(),
        execution_type: "resume".to_string(),
        resume_reason: Some("api_resume".to_string()),
    };

    let mut metadata = HashMap::new();
    metadata.insert("function_result".to_string(), resume_data);

    let job_id = scheduler
        .schedule_flow_job(repo, job_type, metadata)
        .await?;

    tracing::info!(
        job_id = %job_id,
        instance_id = %instance_id,
        "Queued flow instance resume"
    );

    Ok(FlowRunResult {
        instance_id: instance_id.to_string(),
        job_id,
    })
}

/// Get the current status of a flow instance.
pub async fn get_instance_status<S: Storage>(
    storage: &S,
    repo: &str,
    instance_id: &str,
) -> Result<FlowInstanceStatus, FlowError> {
    let (_node, instance) = load_instance(storage, repo, instance_id).await?;

    Ok(FlowInstanceStatus {
        id: instance.id,
        status: instance.status.as_str().to_string(),
        variables: instance.variables,
        flow_path: instance.flow_ref,
        started_at: instance.started_at.to_rfc3339(),
        error: instance.error,
    })
}

/// Cancel a running or waiting flow instance.
///
/// Sets the instance status to `Cancelled` and best-effort cancels associated
/// jobs. Returns an error if the instance is already in a terminal state.
pub async fn cancel_instance<S: Storage>(
    storage: &S,
    scheduler: &dyn FlowJobScheduler,
    repo: &str,
    instance_id: &str,
) -> Result<(), FlowError> {
    let (mut node, instance) = load_instance(storage, repo, instance_id).await?;

    if instance.is_terminated() {
        return Err(FlowError::AlreadyTerminated {
            status: instance.status.as_str().to_string(),
        });
    }

    // Update the instance to cancelled
    node.properties.insert(
        "status".to_string(),
        PropertyValue::String(FlowStatus::Cancelled.as_str().to_string()),
    );
    node.properties.insert(
        "completed_at".to_string(),
        PropertyValue::String(chrono::Utc::now().to_rfc3339()),
    );
    node.properties.remove("wait_info");

    let scope = StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, SYSTEM_WORKSPACE);
    storage
        .nodes()
        .update(
            scope,
            node,
            UpdateNodeOptions {
                validate_schema: false,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| FlowError::Other(e.to_string()))?;

    // Best-effort cancel associated jobs
    let _ = scheduler.cancel_flow_jobs(&instance.id).await;

    tracing::info!(instance_id = %instance_id, "Cancelled flow instance");

    Ok(())
}
