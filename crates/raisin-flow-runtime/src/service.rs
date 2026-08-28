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

// NB: there is deliberately no TENANT_ID constant any more. Flow lookups used a
// hardcoded "default" tenant, so on a multi-tenant server every flow resolved
// against the wrong tenant and reported `Flow '/flows/x' not found` even though
// the node existed — for MCP callers AND the editor's own WebSocket calls. The
// tenant is now a parameter on every entry point.
//
// DEFAULT_BRANCH is still a constant: flow DEFINITIONS are main-scoped, and the
// publish engine deliberately runs flows that copy main -> publish. Making
// resolution follow the caller's branch would change that behaviour, so it is
// left alone rather than changed blind.
const DEFAULT_BRANCH: &str = "main";
const FUNCTIONS_WORKSPACE: &str = "functions";
const SYSTEM_WORKSPACE: &str = "raisin:system";
/// Workspace where inbox tasks (and user homes) live.
const INBOX_WORKSPACE: &str = "raisin:access_control";

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
        tenant_id: &str,
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
    tenant_id: &str,
    repo: &str,
    flow_path: &str,
) -> Result<ValidatedFlow, FlowError> {
    let scope = StorageScope::new(tenant_id, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE);
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
    tenant_id: &str,
    repo: &str,
    instance_id: &str,
) -> Result<(raisin_models::nodes::Node, FlowInstance), FlowError> {
    let scope = StorageScope::new(tenant_id, repo, DEFAULT_BRANCH, SYSTEM_WORKSPACE);
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
    tenant_id: &str,
    repo: &str,
    flow_path: &str,
    input: Value,
    actor: String,
    actor_home: Option<String>,
) -> Result<FlowRunResult, FlowError> {
    let flow = load_and_validate_flow(storage, tenant_id, repo, flow_path).await?;

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
    .tenant_id(tenant_id.to_string())
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
        .schedule_flow_job(tenant_id, repo, job_type, metadata)
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
    tenant_id: &str,
    repo: &str,
    flow_path: &str,
    input: Value,
    mut test_config: crate::types::TestRunConfig,
) -> Result<FlowRunResult, FlowError> {
    let flow = load_and_validate_flow(storage, tenant_id, repo, flow_path).await?;

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
    .tenant_id(tenant_id.to_string())
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
        .schedule_flow_job(tenant_id, repo, job_type, metadata)
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
    tenant_id: &str,
    repo: &str,
    instance_id: &str,
    resume_data: Value,
) -> Result<FlowRunResult, FlowError> {
    let (_node, instance) = load_instance(storage, tenant_id, repo, instance_id).await?;

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
        .schedule_flow_job(tenant_id, repo, job_type, metadata)
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
    tenant_id: &str,
    repo: &str,
    instance_id: &str,
) -> Result<FlowInstanceStatus, FlowError> {
    let (_node, instance) = load_instance(storage, tenant_id, repo, instance_id).await?;

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
    tenant_id: &str,
    repo: &str,
    instance_id: &str,
) -> Result<(), FlowError> {
    let (mut node, instance) = load_instance(storage, tenant_id, repo, instance_id).await?;

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

    let scope = StorageScope::new(tenant_id, repo, DEFAULT_BRANCH, SYSTEM_WORKSPACE);
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

    // A cancelled CHILD must tell its parent, exactly as completion, failure
    // and timeout do. Without this a parent parked on a join (a parallel
    // fan-out, a sub-flow) waits forever for a branch that will never report:
    // the child is Cancelled, the parent never learns, and no error surfaces.
    if let Some(parent_instance_id) = &instance.parent_instance_ref {
        let branch_id = instance
            .variables
            .get("__branch_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut metadata = HashMap::new();
        metadata.insert(
            "function_result".to_string(),
            serde_json::json!({
                "child_completed": true,
                "child_instance_id": instance.id,
                "parent_step_id": instance
                    .variables
                    .get("__parent_step_id")
                    .and_then(|v| v.as_str()),
                "branch_id": branch_id,
                "status": FlowStatus::Cancelled.as_str(),
                "output": Value::Null,
                "error": "child flow was cancelled",
            }),
        );

        let job_type = JobType::FlowInstanceExecution {
            instance_id: parent_instance_id.clone(),
            execution_type: "resume".to_string(),
            resume_reason: Some("child_flow_terminal".to_string()),
        };

        match scheduler
            .schedule_flow_job(tenant_id, repo, job_type, metadata)
            .await
        {
            Ok(job_id) => tracing::info!(
                instance_id = %instance_id,
                parent_instance_id = %parent_instance_id,
                job_id = %job_id,
                "Cancelled child flow - queued parent resume"
            ),
            // Best-effort, like every other parent notification: the wait
            // sweeper is the backstop for a lost one.
            Err(e) => tracing::error!(
                instance_id = %instance_id,
                parent_instance_id = %parent_instance_id,
                error = %e,
                "Cancelled child flow but failed to notify parent"
            ),
        }
    }

    tracing::info!(instance_id = %instance_id, "Cancelled flow instance");

    Ok(())
}

// ---------------------------------------------------------------------------
// Inbox task completion
// ---------------------------------------------------------------------------

/// Identity of the caller completing an inbox task.
#[derive(Debug, Clone)]
pub struct TaskCompleter {
    /// User (or agent) identifier
    pub id: String,
    /// Home path (e.g. "/users/alice" or "/agents/support")
    pub home: Option<String>,
    /// Admins may complete any task
    pub is_admin: bool,
}

impl TaskCompleter {
    fn matches_assignee(&self, assignee: &str) -> bool {
        if self.is_admin {
            return true;
        }
        let norm = |s: &str| s.trim_matches('/').to_string();
        let assignee_n = norm(assignee);
        if let Some(home) = &self.home {
            if norm(home) == assignee_n {
                return true;
            }
        }
        // Fall back to comparing the last path segment with the caller id
        assignee_n
            .rsplit('/')
            .next()
            .map(|last| last == self.id)
            .unwrap_or(false)
    }
}

/// Result of completing an inbox task.
#[derive(Debug, Clone, Serialize)]
pub struct TaskCompletionResult {
    pub task_id: String,
    pub task_path: String,
    pub status: String,
    /// Present when the completion resumed a waiting flow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<FlowRunResult>,
}

/// Load an inbox task node by id or path.
async fn load_task_node<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    task_ref: &str,
) -> Result<raisin_models::nodes::Node, FlowError> {
    let scope = StorageScope::new(tenant_id, repo, DEFAULT_BRANCH, INBOX_WORKSPACE);

    let node = if task_ref.starts_with('/') {
        storage
            .nodes()
            .get_by_path(scope, task_ref, None)
            .await
            .map_err(|e| FlowError::Other(e.to_string()))?
    } else {
        storage
            .nodes()
            .get(scope, task_ref, None)
            .await
            .map_err(|e| FlowError::Other(e.to_string()))?
    };

    let node = node
        .ok_or_else(|| FlowError::NodeNotFound(format!("Inbox task '{}' not found", task_ref)))?;

    if node.node_type != "raisin:InboxTask" {
        return Err(FlowError::InvalidDefinition(format!(
            "Node '{}' is not an inbox task (found: {})",
            task_ref, node.node_type
        )));
    }

    Ok(node)
}

fn task_prop_str(node: &raisin_models::nodes::Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn task_prop_usize(node: &raisin_models::nodes::Node, key: &str) -> Option<usize> {
    match node.properties.get(key) {
        Some(PropertyValue::Integer(value)) if *value >= 0 => Some(*value as usize),
        _ => None,
    }
}

fn task_prop_paths(node: &raisin_models::nodes::Node, key: &str) -> Vec<String> {
    match node.properties.get(key) {
        Some(PropertyValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                PropertyValue::String(path) => Some(path.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// A timestamp property, whatever shape it came back in.
///
/// The writer sets `responded_at` as a String, but the type declares it a Date,
/// so a task READ BACK from storage carries `PropertyValue::Date` — and a
/// String-only reader silently reported `null` for every sibling in a group
/// summary while the just-written one looked fine.
fn task_prop_time(node: &raisin_models::nodes::Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(value)) => Some(value.clone()),
        Some(PropertyValue::Date(value)) => Some(value.to_rfc3339()),
        _ => None,
    }
}

fn task_response(node: &raisin_models::nodes::Node) -> Value {
    node.properties
        .get("response")
        .and_then(|value| serde_json::to_value(value).ok())
        .unwrap_or(Value::Null)
}

fn task_qualifies(node: &raisin_models::nodes::Node) -> bool {
    matches!(node.properties.get("qualifies"), Some(PropertyValue::Boolean(true)))
}

fn evaluate_group_response(
    condition: Option<&str>,
    response: &Value,
    previous: &[Value],
) -> Result<bool, FlowError> {
    let Some(condition) = condition.filter(|value| !value.trim().is_empty()) else {
        return Ok(true);
    };
    let context = raisin_rel::EvalContext::from_json(serde_json::json!({
        "response": response,
        "responses": previous,
    }))
    .map_err(|error| FlowError::InvalidNodeConfiguration(format!(
        "Invalid human-task response context: {}", error
    )))?;
    let value = raisin_rel::eval(condition, &context).map_err(|error| {
        FlowError::InvalidNodeConfiguration(format!(
            "Invalid human-task response_condition '{}': {}", condition, error
        ))
    })?;
    Ok(match value {
        raisin_rel::Value::Boolean(value) => value,
        raisin_rel::Value::Null => false,
        raisin_rel::Value::Integer(value) => value != 0,
        raisin_rel::Value::Float(value) => value != 0.0,
        raisin_rel::Value::String(value) => !value.is_empty(),
        raisin_rel::Value::Array(value) => !value.is_empty(),
        raisin_rel::Value::Object(value) => !value.is_empty(),
    })
}

async fn persist_inbox_task<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    task: raisin_models::nodes::Node,
) -> Result<(), FlowError> {
    let task_id = task.id.clone();
    let task_path = task.path.clone();
    let task_type = task.node_type.clone();
    let scope = StorageScope::new(tenant_id, repo, DEFAULT_BRANCH, INBOX_WORKSPACE);
    storage
        .nodes()
        .update(
            scope,
            task,
            UpdateNodeOptions { validate_schema: false, ..Default::default() },
        )
        .await
        .map_err(|error| FlowError::Other(format!("Failed to update task node: {}", error)))?;

    let revision = {
        use raisin_storage::BranchRepository;
        match storage.branches().get_branch(tenant_id, repo, DEFAULT_BRANCH).await {
            Ok(Some(branch)) => branch.head,
            _ => raisin_hlc::HLC::new(0, 0),
        }
    };
    storage.event_bus().publish(raisin_storage::Event::Node(
        raisin_storage::NodeEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo.to_string(),
            branch: DEFAULT_BRANCH.to_string(),
            workspace_id: INBOX_WORKSPACE.to_string(),
            node_id: task_id,
            node_type: Some(task_type),
            revision,
            kind: raisin_storage::NodeEventKind::Updated,
            path: Some(task_path),
            metadata: None,
        },
    ));
    Ok(())
}

/// Complete an inbox task: validates the caller is the assignee, marks the
/// task node completed, and resumes the owning flow (if any).
///
/// This is the single completion path for humans AND agents - `completed_by`
/// records who decided.
///
/// Idempotency: completing an already-completed task returns
/// `FlowError::AlreadyTerminated`.
pub async fn complete_task<S: Storage>(
    storage: &S,
    scheduler: &dyn FlowJobScheduler,
    tenant_id: &str,
    repo: &str,
    task_ref: &str,
    completer: &TaskCompleter,
    response: Value,
) -> Result<TaskCompletionResult, FlowError> {
    let mut task_node = load_task_node(storage, tenant_id, repo, task_ref).await?;
    let task_path = task_node.path.clone();
    let task_id = task_node.id.clone();

    // 1. Status must be pending
    let status = task_prop_str(&task_node, "status").unwrap_or_else(|| "pending".to_string());
    if status != "pending" {
        return Err(FlowError::AlreadyTerminated { status });
    }

    // 2. Caller must be the assignee (or an admin)
    let assignee = task_prop_str(&task_node, "assignee").unwrap_or_default();
    if !completer.matches_assignee(&assignee) {
        return Err(FlowError::PermissionDenied(format!(
            "Task '{}' is assigned to '{}', not '{}'",
            task_id, assignee, completer.id
        )));
    }

    // 3. If the task belongs to a flow, validate the flow is waiting on it
    let flow_instance_id = task_prop_str(&task_node, "flow_instance_id");
    let step_id = task_prop_str(&task_node, "step_id");
    let assignment_group = task_prop_str(&task_node, "assignment_group");
    let assignment_paths = task_prop_paths(&task_node, "assignment_task_paths");

    if let Some(instance_id) = &flow_instance_id {
        let (_node, instance) = load_instance(storage, tenant_id, repo, instance_id).await?;

        if instance.status != FlowStatus::Waiting {
            return Err(FlowError::InvalidStateTransition {
                from: instance.status.as_str().to_string(),
                to: "resumed".to_string(),
            });
        }
        if let Some(step_id) = &step_id {
            if &instance.current_node_id != step_id {
                return Err(FlowError::InvalidStateTransition {
                    from: format!("waiting at '{}'", instance.current_node_id),
                    to: format!("completing task for step '{}'", step_id),
                });
            }
        }
    }

    // A group response can be counted conditionally (for example only an
    // `accept` response qualifies). Evaluate before mutating the task so an
    // invalid authored condition cannot consume a person's response.
    if assignment_group.is_some() {
        let mut previous_responses = Vec::new();
        for path in assignment_paths.iter().filter(|path| *path != &task_path) {
            if let Ok(sibling) = load_task_node(storage, tenant_id, repo, path).await {
                if task_prop_str(&sibling, "status").as_deref() == Some("completed") {
                    previous_responses.push(task_response(&sibling));
                }
            }
        }
        let qualifies = evaluate_group_response(
            task_prop_str(&task_node, "response_condition").as_deref(),
            &response,
            &previous_responses,
        )?;
        task_node
            .properties
            .insert("qualifies".to_string(), PropertyValue::Boolean(qualifies));
    }

    // 4. Mark the task node completed
    let now = chrono::Utc::now().to_rfc3339();
    task_node.properties.insert(
        "status".to_string(),
        PropertyValue::String("completed".to_string()),
    );
    task_node.properties.insert(
        "completed_by".to_string(),
        PropertyValue::String(completer.id.clone()),
    );
    task_node.properties.insert(
        "responded_at".to_string(),
        PropertyValue::String(now.clone()),
    );
    if let Ok(response_prop) =
        serde_json::from_value::<std::collections::HashMap<String, PropertyValue>>(response.clone())
    {
        task_node
            .properties
            .insert("response".to_string(), PropertyValue::Object(response_prop));
    }

    persist_inbox_task(storage, tenant_id, repo, task_node.clone()).await?;

    // 4b. A GROUP assignment resumes its flow ONCE, not once per member.
    //
    // Every member holds a real inbox task, so this completion API is the only
    // place that can tell whether the authored policy (any / all / quorum,
    // narrowed by `response_condition`) is now satisfied — and the only place
    // that can take the losing tasks back out of people's inboxes when it is.
    // Both outcomes are DESIGNABLE: the flow resumes with `group.satisfied`
    // either way, so "first response wins" and "nobody accepted" are two
    // ordinary branches rather than one silent dead end.
    let group_summary = if let Some(group_path) = assignment_group.clone() {
        let required = task_prop_usize(&task_node, "assignment_required").unwrap_or(1);
        let completion = task_prop_str(&task_node, "assignment_completion")
            .unwrap_or_else(|| "any".to_string());

        let mut siblings = Vec::new();
        for path in assignment_paths.iter() {
            if path == &task_path {
                siblings.push(task_node.clone());
            } else if let Ok(node) = load_task_node(storage, tenant_id, repo, path).await {
                siblings.push(node);
            }
        }
        if siblings.is_empty() {
            siblings.push(task_node.clone());
        }

        let total = siblings.len();
        let mut responses = Vec::new();
        let mut pending = Vec::new();
        let mut qualifying = 0usize;
        let mut responded = 0usize;
        for node in &siblings {
            match task_prop_str(node, "status")
                .unwrap_or_else(|| "pending".to_string())
                .as_str()
            {
                "completed" => {
                    responded += 1;
                    let qualifies = task_qualifies(node);
                    if qualifies {
                        qualifying += 1;
                    }
                    responses.push(serde_json::json!({
                        "assignee": task_prop_str(node, "assignee"),
                        "task_path": node.path,
                        "response": task_response(node),
                        "qualifies": qualifies,
                        "completed_by": task_prop_str(node, "completed_by"),
                        "responded_at": task_prop_time(node, "responded_at"),
                    }));
                }
                "pending" => pending.push(node.clone()),
                _ => {}
            }
        }

        let satisfied = qualifying >= required;
        if !satisfied && !pending.is_empty() {
            // Answered, but the policy still needs more qualifying responses.
            // This person's task is done; the flow keeps waiting on the rest.
            tracing::info!(
                task_id = %task_id,
                group = %group_path,
                qualifying,
                required,
                pending = pending.len(),
                "Group task completed; waiting for more responses"
            );
            return Ok(TaskCompletionResult {
                task_id,
                task_path,
                status: "completed".to_string(),
                flow: None,
            });
        }

        for mut node in pending {
            node.properties.insert(
                "status".to_string(),
                PropertyValue::String("cancelled".to_string()),
            );
            node.properties.insert(
                "cancellation_reason".to_string(),
                PropertyValue::String("group_completed".to_string()),
            );
            let cancelled_path = node.path.clone();
            if let Err(error) = persist_inbox_task(storage, tenant_id, repo, node).await {
                // A stale task left pending is confusing but not corrupting:
                // its flow has already moved on and completing it later finds
                // the instance no longer waiting on this step.
                tracing::warn!(
                    task_path = %cancelled_path,
                    error = %error,
                    "Failed to cancel superseded group task"
                );
            }
        }

        Some(serde_json::json!({
            "path": group_path,
            "completion": completion,
            "required": required,
            "total": total,
            "responded": responded,
            "qualifying": qualifying,
            "satisfied": satisfied,
            "responses": responses,
        }))
    } else {
        None
    };

    // The response the flow reads at the top level is the QUALIFYING one, so a
    // first-response-wins branch reads `${steps.ask.action}` exactly as it does
    // for a single assignee. The full picture stays under `group`.
    let response = match &group_summary {
        Some(summary) => summary
            .get("responses")
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|entry| entry.get("qualifies") == Some(&Value::Bool(true)))
            })
            .and_then(|entry| entry.get("response").cloned())
            .filter(|value| !value.is_null())
            .unwrap_or(response),
        None => response,
    };

    // 5. Resume the owning flow with the response
    let flow = if let Some(instance_id) = &flow_instance_id {
        // Merge response with completion metadata so downstream steps can
        // read both `__human_response.action` and `__human_response.completed_by`
        let mut resume_data = match response {
            Value::Object(map) => Value::Object(map),
            other => serde_json::json!({ "value": other }),
        };
        if let Value::Object(ref mut map) = resume_data {
            map.insert(
                "completed_by".to_string(),
                Value::String(completer.id.clone()),
            );
            map.insert("task_path".to_string(), Value::String(task_path.clone()));
            if let Some(summary) = group_summary.clone() {
                map.insert("group".to_string(), summary);
            }
        }

        match resume_flow(
            storage,
            scheduler,
            tenant_id,
            repo,
            instance_id,
            resume_data,
        )
        .await
        {
            Ok(result) => Some(result),
            Err(e) => {
                // Task is completed but the flow resume could not be queued.
                // The wait sweeper / manual resume can recover; surface the error.
                tracing::error!(
                    task_id = %task_id,
                    instance_id = %instance_id,
                    error = %e,
                    "Task completed but failed to queue flow resume"
                );
                return Err(FlowError::Other(format!(
                    "Task '{}' completed but flow resume failed: {}",
                    task_id, e
                )));
            }
        }
    } else {
        None
    };

    tracing::info!(
        task_id = %task_id,
        task_path = %task_path,
        completed_by = %completer.id,
        resumed_flow = ?flow_instance_id,
        "Inbox task completed"
    );

    Ok(TaskCompletionResult {
        task_id,
        task_path,
        status: "completed".to_string(),
        flow,
    })
}

/// List inbox tasks for an assignee, or ALL tasks when `assignee` is None
/// (admin overview).
///
/// Matches by `assignee` property (covers escalated tasks living under
/// another principal's inbox path) and falls back to the task's path prefix.
pub async fn list_inbox_tasks<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    assignee: Option<&str>,
    status_filter: Option<&str>,
) -> Result<Vec<Value>, FlowError> {
    use raisin_storage::ListOptions;

    let scope = StorageScope::new(tenant_id, repo, DEFAULT_BRANCH, INBOX_WORKSPACE);
    let tasks = storage
        .nodes()
        .list_by_type(scope, "raisin:InboxTask", ListOptions::default())
        .await
        .map_err(|e| FlowError::Other(e.to_string()))?;

    let norm = |s: &str| s.trim_matches('/').to_string();
    let assignee_n = assignee.map(norm);

    let mut result: Vec<Value> = tasks
        .into_iter()
        .filter(|task| {
            if let Some(assignee_n) = &assignee_n {
                let task_assignee = match task.properties.get("assignee") {
                    Some(PropertyValue::String(s)) => norm(s),
                    _ => String::new(),
                };
                let matches_assignee = &task_assignee == assignee_n
                    || norm(&task.path).starts_with(&format!("{}/inbox", assignee_n));
                if !matches_assignee {
                    return false;
                }
            }
            if let Some(want) = status_filter {
                match task.properties.get("status") {
                    Some(PropertyValue::String(s)) => s == want,
                    _ => false,
                }
            } else {
                true
            }
        })
        .map(|task| {
            let mut v = serde_json::to_value(&task.properties).unwrap_or(Value::Null);
            if let Value::Object(ref mut map) = v {
                map.insert("id".to_string(), Value::String(task.id.clone()));
                map.insert("path".to_string(), Value::String(task.path.clone()));
            }
            v
        })
        .collect();

    // Sort: pending first, then priority (desc), then due_at (asc)
    result.sort_by(|a, b| {
        let status_rank = |v: &Value| match v.get("status").and_then(|s| s.as_str()) {
            Some("pending") => 0,
            _ => 1,
        };
        let priority = |v: &Value| -(v.get("priority").and_then(|p| p.as_i64()).unwrap_or(3));
        let due = |v: &Value| {
            v.get("due_at")
                .and_then(|d| d.as_str())
                .map(String::from)
                .unwrap_or_else(|| "9999".to_string())
        };
        status_rank(a)
            .cmp(&status_rank(b))
            .then(priority(a).cmp(&priority(b)))
            .then(due(a).cmp(&due(b)))
    });

    Ok(result)
}

/// Get a single inbox task (properties + id/path) by id or path.
pub async fn get_inbox_task<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    task_ref: &str,
) -> Result<Value, FlowError> {
    let node = load_task_node(storage, tenant_id, repo, task_ref).await?;
    let mut v = serde_json::to_value(&node.properties).unwrap_or(Value::Null);
    if let Value::Object(ref mut map) = v {
        map.insert("id".to_string(), Value::String(node.id.clone()));
        map.insert("path".to_string(), Value::String(node.path.clone()));
    }
    Ok(v)
}

/// Inspect a flow instance's wait state (used by transports to gate the
/// generic resume endpoint: human-task waits must go through the validated
/// task-completion path).
pub async fn get_instance_wait_type<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    instance_id: &str,
) -> Result<Option<crate::types::WaitType>, FlowError> {
    let (_node, instance) = load_instance(storage, tenant_id, repo, instance_id).await?;
    Ok(instance.wait_info.map(|w| w.wait_type))
}

#[cfg(test)]
mod tests {
    use super::TaskCompleter;

    fn completer(id: &str, home: Option<&str>, is_admin: bool) -> TaskCompleter {
        TaskCompleter {
            id: id.to_string(),
            home: home.map(|h| h.to_string()),
            is_admin,
        }
    }

    /// The confirmed-correct assignee string for an identity user is the
    /// user HOME path (e.g. "/users/internal/admin-at-example-local"): it matches
    /// the completer's `home`, regardless of leading-slash normalization. This
    /// is the property `raisin.tasks.create` and the human_task step now store,
    /// and the string `complete_task` / `get_inbox_task` authorize against.
    #[test]
    fn matches_assignee_by_home_path() {
        let c = completer(
            "cdc46e95-425d-4816-9da1-9a61776bbcf3",
            Some("/users/internal/admin-at-example-local"),
            false,
        );
        // Exact home path (as stored by the fixed standalone create + human_task).
        assert!(c.matches_assignee("/users/internal/admin-at-example-local"));
        // Normalization: leading slash is not significant on either side.
        assert!(c.matches_assignee("users/internal/admin-at-example-local"));
        // A different assignee is denied.
        assert!(!c.matches_assignee("/users/internal/someone-else"));
    }

    /// The last-path-segment == caller-id fallback authorizes agent/service
    /// assignees that have no home. Crucially, it does NOT fire for an
    /// identity user against their real HOME-path assignee, because that path's
    /// last segment is the slug (`admin-at-example-local`), not the caller's
    /// uuid — which is exactly why the HOME path, not the bare uuid, is the
    /// correct assignee string.
    #[test]
    fn matches_assignee_last_segment_fallback() {
        // An agent whose id equals the assignee path's last segment matches.
        let agent = completer("support", None, false);
        assert!(agent.matches_assignee("/agents/support"));
        assert!(!agent.matches_assignee("/agents/other"));

        // A uuid-id caller (no home) does NOT match the HOME-path assignee via
        // the fallback: the last segment is the slug, not the uuid. Such a
        // caller is only authorized by the home-path comparison
        // (matches_assignee_by_home_path), documenting why the bare uuid fails.
        let uuid_caller = completer("cdc46e95-425d-4816-9da1-9a61776bbcf3", None, false);
        assert!(!uuid_caller.matches_assignee("/users/internal/admin-at-example-local"));
    }

    /// Admins bypass the assignee check entirely.
    #[test]
    fn matches_assignee_admin_bypass() {
        let admin = completer("root", None, true);
        assert!(admin.matches_assignee("/users/anyone"));
        assert!(admin.matches_assignee(""));
    }

    /// An empty assignee (the pre-fix standalone-task shape, which stored no
    /// `assignee` property) is denied for a non-admin — the regression BUG C
    /// fixed by writing the assignee property on create.
    #[test]
    fn matches_assignee_empty_denied_for_non_admin() {
        let c = completer(
            "cdc46e95-425d-4816-9da1-9a61776bbcf3",
            Some("/users/internal/admin-at-example-local"),
            false,
        );
        assert!(!c.matches_assignee(""));
    }
}
