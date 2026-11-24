//! Completing an inbox task is announced like any other write (G65).
//!
//! `service::complete_task` used to write the task through the raw node
//! repository, which emits nothing, and publish an event by hand with no
//! metadata: no node data, no changed property names, and as revision the
//! branch HEAD read after the write (which a concurrent commit may already
//! have moved). A trigger on `raisin:InboxTask` updates was therefore not
//! reliably delivered. The completion is now a partial update through the
//! transaction layer, and its commit announces it.

use async_trait::async_trait;
use raisin_flow_runtime::service::{complete_task, FlowJobScheduler, TaskCompleter};
use raisin_flow_runtime::types::FlowError;
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, Event, EventBus, EventHandler, JobType, NodeEventKind,
    NodeRepository, RepoScope, Storage, StorageScope, WorkspaceRepository,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

const TENANT: &str = "default";
const REPO: &str = "inbox_event";
const BRANCH: &str = "main";
const INBOX_WS: &str = "raisin:access_control";

struct NoScheduler;

#[async_trait]
impl FlowJobScheduler for NoScheduler {
    async fn schedule_flow_job(
        &self,
        _tenant_id: &str,
        _repo: &str,
        _job_type: JobType,
        _metadata: HashMap<String, Value>,
    ) -> Result<String, FlowError> {
        Ok("unused".into())
    }
    async fn cancel_flow_jobs(&self, _instance_id: &str) -> Result<(), FlowError> {
        Ok(())
    }
}

#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<raisin_storage::NodeEvent>>>);

impl EventHandler for Recorder {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        if let Event::Node(ne) = event {
            self.0.lock().unwrap().push(ne.clone());
        }
        Box::pin(async { Ok(()) })
    }
    fn name(&self) -> &str {
        "inbox-recorder"
    }
}

#[tokio::test]
async fn completing_a_task_emits_an_updated_event_with_its_change() {
    let dir = tempfile::TempDir::new().unwrap();
    let storage = raisin_rocksdb::RocksDBStorage::new(dir.path()).unwrap();
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(INBOX_WS.to_string()),
        )
        .await
        .unwrap();

    let assignee = "/users/manager";
    let mut task = Node {
        id: "task-1".into(),
        name: "task-1".into(),
        path: format!("{assignee}/inbox/task-1"),
        node_type: "raisin:InboxTask".into(),
        ..Default::default()
    };
    for (k, v) in [
        ("task_type", "approval"),
        ("title", "Approve"),
        ("status", "pending"),
        ("assignee", assignee),
        ("seen", "yes"),
    ] {
        task.properties
            .insert(k.into(), PropertyValue::String(v.into()));
    }
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            task,
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("seed task");

    let recorder = Recorder::default();
    storage.event_bus().subscribe(Arc::new(recorder.clone()));

    let result = complete_task(
        &storage,
        &NoScheduler,
        TENANT,
        REPO,
        "task-1",
        &TaskCompleter {
            id: "manager".into(),
            home: Some(assignee.into()),
            is_admin: false,
        },
        json!({ "action": "approve" }),
    )
    .await
    .expect("complete");
    assert_eq!(result.status, "completed");

    // Delivery is asynchronous.
    let mut event = None;
    for _ in 0..100 {
        event = recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.node_id == "task-1" && matches!(e.kind, NodeEventKind::Updated))
            .cloned();
        if event.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let event = event.expect("an Updated event for the task");
    assert_eq!(event.workspace_id, INBOX_WS);
    let meta = event.metadata.expect("the commit's metadata");
    let changed: Vec<String> =
        serde_json::from_value(meta.get("changed_properties").cloned().expect("changed")).unwrap();
    assert!(changed.contains(&"status".to_string()), "{changed:?}");
    assert!(changed.contains(&"completed_by".to_string()), "{changed:?}");
    let data = meta.get("node_data").expect("node data");
    assert_eq!(data["properties"]["status"], json!("completed"));

    // Only the completion's properties were written.
    let stored = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, INBOX_WS),
            "task-1",
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.properties.get("seen"),
        Some(&PropertyValue::String("yes".into()))
    );
}
