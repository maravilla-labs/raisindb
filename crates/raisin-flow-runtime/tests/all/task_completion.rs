// SPDX-License-Identifier: BSL-1.1

//! Integration test for the `raisin.tasks.complete` binding path.
//!
//! The function-side binding (`create_task_complete`) is a thin wrapper: it
//! derives a [`TaskCompleter`] from the caller's auth context and delegates to
//! [`raisin_flow_runtime::service::complete_task`] — the SAME path the HTTP
//! inbox uses. The P0 bug was that the OLD binding mutated the task via
//! `svc.create(updated).ok()`: an upsert-via-create that both swallowed the
//! (now-conflicting) create AND never resumed the owning flow, so a parked
//! `human_task` step never advanced.
//!
//! This test pins the fixed behaviour end-to-end against real (in-memory)
//! storage. It mirrors the runtime-level `human_task_waits_and_resumes_with_response`
//! e2e test, but drives completion through `service::complete_task` instead of
//! calling `resume_flow` directly, so the node update AND the assignee
//! validation — the two halves the binding relies on — are exercised.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use raisin_flow_runtime::service::{complete_task, FlowJobScheduler, TaskCompleter};
use raisin_flow_runtime::types::{FlowError, FlowInstance, FlowStatus};
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_storage::{CreateNodeOptions, JobType, NodeRepository, Storage, StorageScope};
use raisin_storage_memory::InMemoryStorage;

const TENANT: &str = "default";
const REPO: &str = "example";
const BRANCH: &str = "main";
/// Flow instances live in `raisin:system` (see `service::load_instance`).
const SYSTEM_WS: &str = "raisin:system";
/// Inbox tasks (and user homes) live in `raisin:access_control`.
const INBOX_WS: &str = "raisin:access_control";

/// Records scheduled flow jobs so a test can assert the resume was queued (with
/// the human response) without a real background job runtime.
#[derive(Default)]
struct CapturingScheduler {
    scheduled: Mutex<Vec<(JobType, HashMap<String, Value>)>>,
}

#[async_trait]
impl FlowJobScheduler for CapturingScheduler {
    async fn schedule_flow_job(
        &self,
        _tenant_id: &str,
        _repo: &str,
        job_type: JobType,
        metadata: HashMap<String, Value>,
    ) -> Result<String, FlowError> {
        self.scheduled.lock().unwrap().push((job_type, metadata));
        Ok("job-under-test".to_string())
    }

    async fn cancel_flow_jobs(&self, _instance_id: &str) -> Result<(), FlowError> {
        Ok(())
    }
}

fn no_validation() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        ..Default::default()
    }
}

/// Seed a `Waiting` flow instance parked at `step_id` into `raisin:system`.
///
/// `service::load_instance` reads the node's property bag straight back into a
/// `FlowInstance`, so the instance is stored as its own serialized properties.
async fn seed_waiting_instance(storage: &InMemoryStorage, instance_id: &str, step_id: &str) {
    let instance = FlowInstance {
        id: instance_id.to_string(),
        version: 1,
        flow_ref: "/flows/approval".to_string(),
        flow_version: 1,
        flow_definition_snapshot: json!({ "nodes": [] }),
        status: FlowStatus::Waiting,
        current_node_id: step_id.to_string(),
        wait_info: None,
        variables: json!({}),
        input: json!({}),
        output: None,
        compensation_stack: Vec::new(),
        error: None,
        retry_count: 0,
        started_at: chrono::Utc::now(),
        completed_at: None,
        parent_instance_ref: None,
        metrics: Default::default(),
        test_config: None,
    };

    let props: HashMap<String, PropertyValue> =
        serde_json::from_value(serde_json::to_value(&instance).expect("instance -> json"))
            .expect("instance json -> property bag");

    let node = Node {
        id: instance_id.to_string(),
        name: instance_id.to_string(),
        path: format!("/flows/instances/{}", instance_id),
        node_type: "raisin:FlowInstance".to_string(),
        properties: props,
        ..Default::default()
    };
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, SYSTEM_WS),
            node,
            no_validation(),
        )
        .await
        .expect("seed flow instance");
}

/// Seed a pending, flow-owned inbox task assigned to `assignee`.
async fn seed_pending_task(
    storage: &InMemoryStorage,
    task_id: &str,
    assignee: &str,
    instance_id: &str,
    step_id: &str,
) -> String {
    let path = format!("{}/inbox/{}", assignee, task_id);
    let mut node = Node {
        id: task_id.to_string(),
        name: task_id.to_string(),
        path: path.clone(),
        node_type: "raisin:InboxTask".to_string(),
        ..Default::default()
    };
    for (k, v) in [
        ("task_type", "approval"),
        ("title", "Approve order ORD-7"),
        ("status", "pending"),
        ("assignee", assignee),
        ("flow_instance_id", instance_id),
        ("step_id", step_id),
    ] {
        node.properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            node,
            no_validation(),
        )
        .await
        .expect("seed inbox task");
    path
}

async fn task_status(storage: &InMemoryStorage, task_id: &str) -> String {
    let node = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            task_id,
            None,
        )
        .await
        .expect("get task")
        .expect("task exists");
    match node.properties.get("status") {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("unexpected task status property: {:?}", other),
    }
}

/// Happy path: completing a flow-owned task marks the node completed AND queues
/// the owning flow's resume with the human response. This is exactly what the
/// old create-upsert binding failed to do (task never changed, flow never
/// resumed).
#[tokio::test]
async fn complete_resumes_parked_human_task_flow() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();

    let instance_id = "flow-approval-1";
    let assignee = "/users/manager";
    let step_id = "approve";
    let task_id = "task-approve-1";

    seed_waiting_instance(&storage, instance_id, step_id).await;
    let task_path = seed_pending_task(&storage, task_id, assignee, instance_id, step_id).await;

    // The assignee (identified by their home path) completes the task.
    let completer = TaskCompleter {
        id: "manager".to_string(),
        home: Some(assignee.to_string()),
        is_admin: false,
    };

    let result = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        task_id,
        &completer,
        json!({ "action": "approve" }),
    )
    .await
    .expect("completion should succeed for the assignee");

    // 1. The result reports the task completed and a resumed flow.
    assert_eq!(result.status, "completed");
    let flow = result
        .flow
        .expect("a flow-owned task must report its resume");
    assert_eq!(flow.instance_id, instance_id);

    // 2. The task NODE was actually updated to completed (the old
    //    `svc.create(updated).ok()` silently dropped this).
    assert_eq!(task_status(&storage, task_id).await, "completed");

    // 3. Exactly one resume job was queued for THIS instance, carrying the human
    //    response (merged with completion metadata) as `function_result`.
    let scheduled = scheduler.scheduled.lock().unwrap();
    assert_eq!(scheduled.len(), 1, "exactly one resume job must be queued");
    let (job_type, metadata) = &scheduled[0];
    match job_type {
        JobType::FlowInstanceExecution {
            instance_id: iid,
            execution_type,
            ..
        } => {
            assert_eq!(iid, instance_id);
            assert_eq!(execution_type, "resume");
        }
        other => panic!(
            "expected a FlowInstanceExecution resume job, got {:?}",
            other
        ),
    }
    let resume = &metadata["function_result"];
    assert_eq!(
        resume["action"],
        json!("approve"),
        "response reaches the flow"
    );
    assert_eq!(resume["completed_by"], json!("manager"));
    assert_eq!(resume["task_path"], json!(task_path));
}

/// Assignee validation is preserved: a different, non-admin caller cannot
/// complete the task — and on denial nothing mutates and no flow resumes.
#[tokio::test]
async fn complete_denies_non_assignee_and_does_not_resume() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();

    let instance_id = "flow-approval-2";
    let assignee = "/users/manager";
    let step_id = "approve";
    let task_id = "task-approve-2";

    seed_waiting_instance(&storage, instance_id, step_id).await;
    seed_pending_task(&storage, task_id, assignee, instance_id, step_id).await;

    let intruder = TaskCompleter {
        id: "someone-else".to_string(),
        home: Some("/users/someone-else".to_string()),
        is_admin: false,
    };

    let err = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        task_id,
        &intruder,
        json!({ "action": "approve" }),
    )
    .await
    .expect_err("a non-assignee must be denied");
    assert!(
        matches!(err, FlowError::PermissionDenied(_)),
        "expected PermissionDenied, got {:?}",
        err
    );

    // The task stays pending and NO resume was queued.
    assert_eq!(task_status(&storage, task_id).await, "pending");
    assert!(
        scheduler.scheduled.lock().unwrap().is_empty(),
        "a denied completion must not queue a flow resume"
    );
}

/// Idempotency: a second completion of the now-completed task is rejected by the
/// status guard, so a double-click cannot double-resume the flow.
#[tokio::test]
async fn complete_is_idempotent_after_first_completion() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();

    let instance_id = "flow-approval-3";
    let assignee = "/users/manager";
    let step_id = "approve";
    let task_id = "task-approve-3";

    seed_waiting_instance(&storage, instance_id, step_id).await;
    seed_pending_task(&storage, task_id, assignee, instance_id, step_id).await;

    let completer = TaskCompleter {
        id: "manager".to_string(),
        home: Some(assignee.to_string()),
        is_admin: false,
    };

    complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        task_id,
        &completer,
        json!({ "action": "approve" }),
    )
    .await
    .expect("first completion succeeds");

    let err = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        task_id,
        &completer,
        json!({ "action": "approve" }),
    )
    .await
    .expect_err("second completion of a completed task must be rejected");
    assert!(
        matches!(err, FlowError::AlreadyTerminated { .. }),
        "expected AlreadyTerminated, got {:?}",
        err
    );

    // Only the first completion queued a resume.
    assert_eq!(
        scheduler.scheduled.lock().unwrap().len(),
        1,
        "the rejected re-completion must not queue a second resume"
    );
}
