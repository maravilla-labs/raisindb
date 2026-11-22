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

use raisin_flow_runtime::assignment;
use raisin_flow_runtime::service::{complete_task, FlowJobScheduler, TaskCompleter};
use raisin_flow_runtime::types::{FlowError, FlowInstance, FlowStatus, WaitInfo, WaitType};
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
    seed_waiting_instance_on(storage, instance_id, step_id, None).await
}

/// [`seed_waiting_instance`] with the wait naming the task it waits on, as
/// `handle_wait_result` records it (`wait_info.target_path`).
async fn seed_waiting_instance_on(
    storage: &InMemoryStorage,
    instance_id: &str,
    step_id: &str,
    target_path: Option<&str>,
) {
    let instance = FlowInstance {
        id: instance_id.to_string(),
        version: 1,
        flow_ref: "/flows/approval".to_string(),
        flow_version: 1,
        flow_definition_snapshot: json!({ "nodes": [] }),
        status: FlowStatus::Waiting,
        current_node_id: step_id.to_string(),
        wait_info: target_path.map(|path| WaitInfo {
            subscription_id: "sub-1".to_string(),
            wait_type: WaitType::HumanTask,
            target_path: Some(path.to_string()),
            expected_event: None,
            timeout_at: None,
        }),
        variables: json!({}),
        input: json!({}),
        output: None,
        compensation_stack: Vec::new(),
        error: None,
        retry_count: 0,
        checkpoints: Vec::new(),
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

// --- Group assignment ------------------------------------------------------
//
// A human_task addressed to a `raisin:Group` is expanded by the step handler
// into one correlated task per member, all pointing at the SAME parked flow
// step. Completion is therefore the only place that can decide whether the
// authored policy (any / all / quorum, narrowed by an optional REL
// `response_condition`) is satisfied — and the only place that can take the
// losing tasks back out of the other members' inboxes when it is.
//
// The invariant these tests pin: the flow resumes EXACTLY ONCE per group wait,
// and it resumes either way — a group where nobody qualified comes back with
// `group.satisfied == false` so "nobody accepted" is a designable branch rather
// than a flow parked forever.

/// Seed one group wait the way the human_task step writes it: the system-only
/// `raisin:TaskAssignment` record holding the policy, the member tasks and the
/// flow linkage, and one pending task per member that only names the record.
async fn seed_group_tasks(
    storage: &InMemoryStorage,
    instance_id: &str,
    step_id: &str,
    members: &[&str],
    completion: &str,
    required: usize,
    response_condition: Option<&str>,
) -> Vec<String> {
    let task_ids: Vec<String> = members
        .iter()
        .enumerate()
        .map(|(index, _)| format!("group-task-{}", index))
        .collect();
    let task_paths: Vec<String> = members
        .iter()
        .zip(task_ids.iter())
        .map(|(member, id)| format!("{}/inbox/{}", member, id))
        .collect();

    let policy = assignment::Policy {
        completion: completion.to_string(),
        required,
        response_condition: response_condition.map(String::from),
    };
    let record = assignment::record_properties(
        "/groups/dispatchers",
        &policy,
        &task_paths,
        Some(instance_id),
        Some(step_id),
        None,
    );
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            Node {
                id: "assignment-1".to_string(),
                name: "assignment-1".to_string(),
                path: assignment::record_path("assignment-1"),
                node_type: assignment::RECORD_TYPE.to_string(),
                properties: serde_json::from_value(record).unwrap(),
                ..Default::default()
            },
            no_validation(),
        )
        .await
        .expect("seed record");

    for ((member, task_id), task_path) in members.iter().zip(task_ids.iter()).zip(task_paths.iter())
    {
        let base = json!({
            "task_type": "approval",
            "title": "Can anyone take this shift?",
            "flow_instance_id": instance_id,
            "step_id": step_id,
        });
        let props = assignment::member_task_properties(
            &base,
            member,
            "assignment-1",
            "/groups/dispatchers",
            &policy,
            members.len(),
        );
        storage
            .nodes()
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
                Node {
                    id: task_id.clone(),
                    name: task_id.clone(),
                    path: task_path.clone(),
                    node_type: "raisin:InboxTask".to_string(),
                    properties: serde_json::from_value(props).unwrap(),
                    ..Default::default()
                },
                no_validation(),
            )
            .await
            .expect("seed group task");
    }
    task_ids
}

fn completer_for(member: &str) -> TaskCompleter {
    TaskCompleter {
        id: member.rsplit('/').next().unwrap().to_string(),
        home: Some(member.to_string()),
        is_admin: false,
    }
}

/// First response wins: one member accepting resumes the flow once, cancels the
/// other members' tasks, and hands the flow both the winning response and the
/// whole picture under `group`.
#[tokio::test]
async fn group_any_resumes_once_and_cancels_the_losers() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben", "/users/cara"];

    seed_waiting_instance(&storage, "flow-dispatch-1", "ask_dispatchers").await;
    let task_ids = seed_group_tasks(
        &storage,
        "flow-dispatch-1",
        "ask_dispatchers",
        &members,
        "any",
        1,
        None,
    )
    .await;

    let result = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[1],
        &completer_for(members[1]),
        json!({ "action": "accept", "note": "I can cover it" }),
    )
    .await
    .expect("the assignee may complete their own group task");

    assert_eq!(result.status, "completed");
    assert!(result.flow.is_some(), "the group wait resumes its flow");
    assert_eq!(task_status(&storage, &task_ids[1]).await, "completed");
    assert_eq!(
        task_status(&storage, &task_ids[0]).await,
        "cancelled",
        "a superseded group task must leave the other members' inboxes"
    );
    assert_eq!(task_status(&storage, &task_ids[2]).await, "cancelled");

    let scheduled = scheduler.scheduled.lock().unwrap();
    assert_eq!(scheduled.len(), 1, "exactly one resume for the whole group");
    let resume = &scheduled[0].1["function_result"];
    assert_eq!(resume["action"], json!("accept"));
    assert_eq!(resume["group"]["satisfied"], json!(true));
    assert_eq!(resume["group"]["path"], json!("/groups/dispatchers"));
    assert_eq!(resume["group"]["total"], json!(3));
    assert_eq!(resume["group"]["responded"], json!(1));
    assert_eq!(resume["group"]["qualifying"], json!(1));
    assert_eq!(
        resume["group"]["responses"][0]["assignee"],
        json!("/users/ben"),
        "the flow can see WHO answered, not just what they said"
    );
}

/// A `response_condition` decides which answers COUNT. A decline is a real
/// completed task, but it does not satisfy the wait — so the flow keeps waiting
/// and the remaining members keep their tasks.
#[tokio::test]
async fn group_non_qualifying_response_does_not_resume_the_flow() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben"];

    seed_waiting_instance(&storage, "flow-dispatch-2", "ask_dispatchers").await;
    let task_ids = seed_group_tasks(
        &storage,
        "flow-dispatch-2",
        "ask_dispatchers",
        &members,
        "any",
        1,
        Some("response.action == 'accept'"),
    )
    .await;

    let result = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[0],
        &completer_for(members[0]),
        json!({ "action": "decline" }),
    )
    .await
    .expect("declining is a valid completion");

    assert_eq!(result.status, "completed");
    assert!(
        result.flow.is_none(),
        "a non-qualifying response must not resume the flow"
    );
    assert_eq!(task_status(&storage, &task_ids[0]).await, "completed");
    assert_eq!(
        task_status(&storage, &task_ids[1]).await,
        "pending",
        "the other member is still being asked"
    );
    assert!(scheduler.scheduled.lock().unwrap().is_empty());
}

/// Nobody qualified. Once the last member has answered, the flow resumes with
/// `group.satisfied == false` — the branch an author writes for "we found
/// nobody" — instead of parking forever.
#[tokio::test]
async fn group_resumes_unsatisfied_when_every_member_declined() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben"];

    seed_waiting_instance(&storage, "flow-dispatch-3", "ask_dispatchers").await;
    let task_ids = seed_group_tasks(
        &storage,
        "flow-dispatch-3",
        "ask_dispatchers",
        &members,
        "any",
        1,
        Some("response.action == 'accept'"),
    )
    .await;

    for (task_id, member) in task_ids.iter().zip(members.iter()) {
        complete_task(
            &storage,
            &scheduler,
            TENANT,
            REPO,
            task_id,
            &completer_for(member),
            json!({ "action": "decline" }),
        )
        .await
        .expect("declining is a valid completion");
    }

    let scheduled = scheduler.scheduled.lock().unwrap();
    assert_eq!(
        scheduled.len(),
        1,
        "only the last answer resumes, and it resumes once"
    );
    let resume = &scheduled[0].1["function_result"];
    assert_eq!(resume["group"]["satisfied"], json!(false));
    assert_eq!(resume["group"]["qualifying"], json!(0));
    assert_eq!(resume["group"]["responded"], json!(2));
    assert_eq!(
        resume["action"],
        json!("decline"),
        "with nothing qualifying, the last real response still reaches the flow"
    );
}

/// `quorum` counts only qualifying responses, and only the response that
/// REACHES the quorum resumes the flow.
#[tokio::test]
async fn group_quorum_resumes_on_the_response_that_reaches_it() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben", "/users/cara"];

    seed_waiting_instance(&storage, "flow-dispatch-4", "ask_dispatchers").await;
    let task_ids = seed_group_tasks(
        &storage,
        "flow-dispatch-4",
        "ask_dispatchers",
        &members,
        "quorum",
        2,
        Some("response.action == 'accept'"),
    )
    .await;

    let first = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[0],
        &completer_for(members[0]),
        json!({ "action": "accept" }),
    )
    .await
    .expect("first acceptance");
    assert!(first.flow.is_none(), "one of two is not a quorum");

    let second = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[1],
        &completer_for(members[1]),
        json!({ "action": "accept" }),
    )
    .await
    .expect("second acceptance");
    assert!(
        second.flow.is_some(),
        "the second acceptance reaches quorum"
    );
    assert_eq!(task_status(&storage, &task_ids[2]).await, "cancelled");

    let scheduled = scheduler.scheduled.lock().unwrap();
    assert_eq!(scheduled.len(), 1);
    let resume = &scheduled[0].1["function_result"];
    assert_eq!(resume["group"]["qualifying"], json!(2));
    assert_eq!(resume["group"]["satisfied"], json!(true));
    assert_eq!(
        resume["group"]["responses"].as_array().unwrap().len(),
        2,
        "every counted answer is carried back, not just the last one"
    );
}

// --- A person's copy is never authority -----------------------------------
//
// A person may update anything in their own inbox. These pin that editing a
// copy can neither reach other people's tasks nor answer a wait it was not
// created for (docs/plans/group-messaging-and-tasks.md).

async fn set_task_props(storage: &InMemoryStorage, task_id: &str, fields: Value) {
    let scope = StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS);
    let mut node = storage
        .nodes()
        .get(scope, task_id, None)
        .await
        .unwrap()
        .expect("task exists");
    let bag: HashMap<String, PropertyValue> = serde_json::from_value(fields).unwrap();
    node.properties.extend(bag);
    storage
        .nodes()
        .update(scope, node, Default::default())
        .await
        .expect("edit own copy");
}

async fn node_prop(storage: &InMemoryStorage, path: &str, key: &str) -> Option<Value> {
    let node = storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            path,
            None,
        )
        .await
        .unwrap()?;
    node.properties
        .get(key)
        .and_then(|v| serde_json::to_value(v).ok())
}

/// A copy that names a record it is not part of is refused, and nothing moves.
#[tokio::test]
async fn copy_naming_a_foreign_assignment_is_refused() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben"];
    seed_waiting_instance(&storage, "flow-forge-1", "ask").await;
    let task_ids =
        seed_group_tasks(&storage, "flow-forge-1", "ask", &members, "any", 1, None).await;

    // Mallory's own standalone task, edited to claim the dispatchers' record.
    seed_pending_task(
        &storage,
        "mallory-task",
        "/users/mallory",
        "flow-forge-1",
        "ask",
    )
    .await;
    set_task_props(
        &storage,
        "mallory-task",
        json!({ "assignment_id": "assignment-1" }),
    )
    .await;

    let err = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        "mallory-task",
        &completer_for("/users/mallory"),
        json!({ "action": "accept" }),
    )
    .await
    .expect_err("a task outside the record must not answer it");
    assert!(
        matches!(err, FlowError::PermissionDenied(_)),
        "got {:?}",
        err
    );
    assert_eq!(task_status(&storage, &task_ids[0]).await, "pending");
    assert_eq!(task_status(&storage, &task_ids[1]).await, "pending");
    assert!(scheduler.scheduled.lock().unwrap().is_empty());
}

/// Task paths written into a copy (the pre-record shape) are ignored: the copy
/// is an ordinary single task, and other people's tasks are untouched.
#[tokio::test]
async fn task_paths_written_into_a_copy_are_ignored() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben"];
    seed_waiting_instance(&storage, "flow-forge-2", "ask").await;
    let task_ids =
        seed_group_tasks(&storage, "flow-forge-2", "ask", &members, "any", 1, None).await;

    let path =
        seed_pending_task(&storage, "mallory-own", "/users/mallory", "other-flow", "x").await;
    set_task_props(
        &storage,
        "mallory-own",
        json!({
            "flow_instance_id": null,
            "step_id": null,
            "assignment_group": "/groups/dispatchers",
            "assignment_completion": "any",
            "assignment_required": 1,
            "assignment_task_paths": [path, "/users/ana/inbox/group-task-0", "/users/ben/inbox/group-task-1"],
        }),
    )
    .await;

    complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        "mallory-own",
        &completer_for("/users/mallory"),
        json!({ "action": "done" }),
    )
    .await
    .expect("her own task completes");
    assert_eq!(task_status(&storage, &task_ids[0]).await, "pending");
    assert_eq!(task_status(&storage, &task_ids[1]).await, "pending");
    assert!(scheduler.scheduled.lock().unwrap().is_empty());
}

/// A single task answers only the wait that was created for it: pointing your
/// own task at another flow's human step is refused.
#[tokio::test]
async fn single_task_cannot_answer_another_flows_wait() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    seed_waiting_instance_on(
        &storage,
        "flow-payout",
        "approve",
        Some("/users/manager/inbox/task-payout"),
    )
    .await;
    seed_pending_task(
        &storage,
        "task-payout",
        "/users/manager",
        "flow-payout",
        "approve",
    )
    .await;

    seed_pending_task(&storage, "mallory-own", "/users/mallory", "mine", "x").await;
    set_task_props(
        &storage,
        "mallory-own",
        json!({ "flow_instance_id": "flow-payout", "step_id": "approve" }),
    )
    .await;

    let err = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        "mallory-own",
        &completer_for("/users/mallory"),
        json!({ "action": "approve" }),
    )
    .await
    .expect_err("another flow's wait is not hers to answer");
    assert!(
        matches!(err, FlowError::PermissionDenied(_)),
        "got {:?}",
        err
    );
    assert!(scheduler.scheduled.lock().unwrap().is_empty());
    assert_eq!(task_status(&storage, "task-payout").await, "pending");

    // The real assignee still can.
    complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        "task-payout",
        &completer_for("/users/manager"),
        json!({ "action": "approve" }),
    )
    .await
    .expect("the task the flow waits on resumes it");
    assert_eq!(scheduler.scheduled.lock().unwrap().len(), 1);
}

/// An answer counts once: setting your own copy back to pending and answering
/// again is refused, so nobody can fill a quorum alone.
#[tokio::test]
async fn a_member_answers_once_even_after_reopening_their_copy() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben", "/users/cara"];
    seed_waiting_instance(&storage, "flow-quorum", "helpers").await;
    let task_ids = seed_group_tasks(
        &storage,
        "flow-quorum",
        "helpers",
        &members,
        "quorum",
        2,
        None,
    )
    .await;

    complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[0],
        &completer_for(members[0]),
        json!({ "action": "yes" }),
    )
    .await
    .expect("first answer");
    set_task_props(&storage, &task_ids[0], json!({ "status": "pending" })).await;
    let err = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[0],
        &completer_for(members[0]),
        json!({ "action": "yes" }),
    )
    .await
    .expect_err("a second answer from the same task");
    assert!(
        matches!(err, FlowError::AlreadyTerminated { .. }),
        "got {:?}",
        err
    );
    assert!(
        scheduler.scheduled.lock().unwrap().is_empty(),
        "one of two is not a quorum"
    );
}

/// While a quorum is open, every member's task shows how many answered.
#[tokio::test]
async fn progress_is_shown_on_every_members_task() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben", "/users/cara"];
    seed_waiting_instance(&storage, "flow-progress", "helpers").await;
    let task_ids = seed_group_tasks(
        &storage,
        "flow-progress",
        "helpers",
        &members,
        "quorum",
        2,
        None,
    )
    .await;

    complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[0],
        &completer_for(members[0]),
        json!({ "action": "yes" }),
    )
    .await
    .unwrap();
    for member in &members[1..] {
        let index = members.iter().position(|m| m == member).unwrap();
        let path = format!("{}/inbox/{}", member, task_ids[index]);
        assert_eq!(
            node_prop(&storage, &path, "assignment_responded").await,
            Some(json!(1))
        );
        assert_eq!(
            node_prop(&storage, &path, "assignment_qualifying").await,
            Some(json!(1))
        );
        assert_eq!(
            node_prop(&storage, &path, "status").await,
            Some(json!("pending"))
        );
    }
}

/// Once closed, a member who reopens their cancelled copy cannot answer.
#[tokio::test]
async fn a_closed_assignment_takes_no_more_answers() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = ["/users/ana", "/users/ben"];
    seed_waiting_instance(&storage, "flow-closed", "ask").await;
    let task_ids = seed_group_tasks(&storage, "flow-closed", "ask", &members, "any", 1, None).await;

    complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[0],
        &completer_for(members[0]),
        json!({ "action": "mine" }),
    )
    .await
    .unwrap();
    set_task_props(&storage, &task_ids[1], json!({ "status": "pending" })).await;
    let err = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &task_ids[1],
        &completer_for(members[1]),
        json!({ "action": "mine" }),
    )
    .await
    .expect_err("the assignment is closed");
    assert!(
        matches!(err, FlowError::AlreadyTerminated { .. }),
        "got {:?}",
        err
    );
    assert_eq!(scheduler.scheduled.lock().unwrap().len(), 1);
}

// --- Group tasks outside workflows ------------------------------------------

async fn seed_user(storage: &InMemoryStorage, home: &str, name: &str, groups: &[&str]) {
    let mut props = json!({ "display_name": name, "email": format!("{}@example.com", name) });
    if !groups.is_empty() {
        props["groups"] = json!(groups);
    }
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            Node {
                id: format!("user-{}", home.rsplit('/').next().unwrap()),
                name: home.rsplit('/').next().unwrap().to_string(),
                path: home.to_string(),
                node_type: "raisin:User".to_string(),
                properties: serde_json::from_value(props).unwrap(),
                ..Default::default()
            },
            no_validation(),
        )
        .await
        .expect("seed user");
}

/// "Someone bring toilet paper": a group task without a workflow. Every member
/// gets a task in their own inbox; the first answer closes it, the others'
/// tasks are cancelled and show who took it.
#[tokio::test]
async fn assignment_outside_a_flow_closes_and_names_who_took_it() {
    let storage = InMemoryStorage::default();
    let scheduler = CapturingScheduler::default();
    let members = [
        "/users/internal/u_ana",
        "/users/internal/u_ben",
        "/users/internal/u_cara",
    ];
    for (home, name) in members.iter().zip(["Anna B.", "Ben K.", "Cara M."]) {
        seed_user(&storage, home, name, &[]).await;
    }

    let created = assignment::create_assignment(
        &storage,
        TENANT,
        REPO,
        assignment::NewAssignment {
            assignment_id: None,
            group: assignment::GroupMembers {
                group_path: "/circles/course-piano-a".to_string(),
                members: members.iter().map(|m| m.to_string()).collect(),
            },
            policy: assignment::Policy::new(Some("any"), None, None, members.len()).unwrap(),
            task: json!({
                "task_type": "bring_item",
                "title": "Bring toilet paper on Saturday",
                "conversation_id": "grp-course-piano-a",
            }),
            flow_instance_id: None,
            step_id: None,
            assigned_by: Some(json!({ "key": "u_teacher", "display_name": "Mrs. Field" })),
        },
    )
    .await
    .expect("assign to the circle");
    assert!(created.created);
    assert_eq!(created.task_paths.len(), 3);
    for (path, home) in created.task_paths.iter().zip(members.iter()) {
        assert!(
            path.starts_with(&format!("{}/inbox/", home)),
            "{} is in {}'s inbox",
            path,
            home
        );
        assert_eq!(
            node_prop(&storage, path, "status").await,
            Some(json!("pending"))
        );
        assert_eq!(
            node_prop(&storage, path, "assignee").await,
            Some(json!(home))
        );
        assert_eq!(
            node_prop(&storage, path, "conversation_id").await,
            Some(json!("grp-course-piano-a"))
        );
        assert_eq!(
            node_prop(&storage, path, "assigned_by").await,
            Some(json!({ "key": "u_teacher", "display_name": "Mrs. Field" }))
        );
        assert_eq!(
            node_prop(&storage, path, "assignment_task_paths").await,
            None,
            "a copy names nobody else's inbox"
        );
    }

    let ben_task = storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            &created.task_paths[1],
            None,
        )
        .await
        .unwrap()
        .unwrap();
    let result = complete_task(
        &storage,
        &scheduler,
        TENANT,
        REPO,
        &ben_task.id,
        &completer_for(members[1]),
        json!({ "action": "take" }),
    )
    .await
    .expect("Ben takes it");
    assert!(result.flow.is_none(), "no workflow waits on it");
    assert!(scheduler.scheduled.lock().unwrap().is_empty());

    assert_eq!(
        node_prop(&storage, &created.task_paths[1], "status").await,
        Some(json!("completed"))
    );
    for path in [&created.task_paths[0], &created.task_paths[2]] {
        assert_eq!(
            node_prop(&storage, path, "status").await,
            Some(json!("cancelled"))
        );
        assert_eq!(
            node_prop(&storage, path, "cancellation_reason").await,
            Some(json!("group_completed"))
        );
        assert_eq!(
            node_prop(&storage, path, "resolved_by_names").await,
            Some(json!(["Ben K."]))
        );
        assert_eq!(
            node_prop(&storage, path, "assignment_status").await,
            Some(json!("satisfied"))
        );
    }
    let record = assignment::record_path(&created.assignment_id);
    assert_eq!(
        node_prop(&storage, &record, "status").await,
        Some(json!("satisfied"))
    );
}

/// A redelivered creation (a trigger retry with the same assignment id) assigns once.
#[tokio::test]
async fn a_redelivered_assignment_assigns_once() {
    let storage = InMemoryStorage::default();
    let members = ["/users/internal/u_ana", "/users/internal/u_ben"];
    let new = || assignment::NewAssignment {
        assignment_id: Some("ga-msg-123".to_string()),
        group: assignment::GroupMembers {
            group_path: "/circles/club".to_string(),
            members: members.iter().map(|m| m.to_string()).collect(),
        },
        policy: assignment::Policy::new(Some("all"), None, None, members.len()).unwrap(),
        task: json!({ "task_type": "confirm_attendance", "title": "Are you coming?" }),
        flow_instance_id: None,
        step_id: None,
        assigned_by: None,
    };
    let first = assignment::create_assignment(&storage, TENANT, REPO, new())
        .await
        .unwrap();
    let again = assignment::create_assignment(&storage, TENANT, REPO, new())
        .await
        .unwrap();
    assert!(first.created);
    assert!(!again.created);
    assert_eq!(first.task_paths, again.task_paths);
    let tasks = storage
        .nodes()
        .list_by_type(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            "raisin:InboxTask",
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(tasks.len(), 2, "one task per member, not two");
}

/// `all` needs every member; required is the member count.
#[test]
fn policies_are_checked_against_the_members() {
    assert_eq!(
        assignment::Policy::new(Some("all"), None, None, 4)
            .unwrap()
            .required,
        4
    );
    assert_eq!(
        assignment::Policy::new(None, None, None, 4)
            .unwrap()
            .completion,
        "any"
    );
    assert_eq!(
        assignment::Policy::new(Some("quorum"), Some(3), None, 4)
            .unwrap()
            .required,
        3
    );
    assert!(assignment::Policy::new(Some("quorum"), Some(5), None, 4).is_err());
    assert!(assignment::Policy::new(Some("most"), None, None, 4).is_err());
    assert!(assignment::Policy::new(Some("any"), None, None, 0).is_err());
}

/// A permission group resolves to the users that carry it.
#[tokio::test]
async fn a_group_resolves_to_its_users() {
    let storage = InMemoryStorage::default();
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            Node {
                id: "group-office".to_string(),
                name: "office".to_string(),
                path: "/groups/office".to_string(),
                node_type: "raisin:Group".to_string(),
                properties: serde_json::from_value(
                    json!({ "group_id": "office", "name": "Office" }),
                )
                .unwrap(),
                ..Default::default()
            },
            no_validation(),
        )
        .await
        .unwrap();
    seed_user(&storage, "/users/internal/u_ana", "Ana", &["office"]).await;
    seed_user(&storage, "/users/internal/u_ben", "Ben", &["teachers"]).await;
    seed_user(
        &storage,
        "/users/internal/u_cara",
        "Cara",
        &["/groups/office"],
    )
    .await;

    let group = assignment::resolve_group(&storage, TENANT, REPO, "/groups/office")
        .await
        .unwrap()
        .expect("a group");
    assert_eq!(
        group.members,
        vec!["/users/internal/u_ana", "/users/internal/u_cara"]
    );
    assert!(
        assignment::resolve_group(&storage, TENANT, REPO, "/users/internal/u_ana")
            .await
            .unwrap()
            .is_none(),
        "a user is not a group"
    );
}
