//! Every committed write that changes a property emits an `Updated` event
//! whose `changed_properties` names it — also when the write lands within
//! milliseconds after another write of the same node, and when both race.
//! (`filters.changed_properties` triggers decide from exactly this list.)
//!
//! Before partial updates, racing merges produced EXTRA status events: a
//! merge evaluated against a stale row wrote the old status back, its event
//! named `status` (the revert), and a handler that read the live node found
//! the change gone — "the trigger did not fire" from the handler's side. The
//! racing test counts exactly one `status` event per status write.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, Event, EventHandler, NodeEventKind,
    NodeTypeRepository, RepoScope, Storage, WorkspaceRepository,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TENANT: &str = "t_changed";
const REPO: &str = "r_changed";
const BRANCH: &str = "main";
const WS: &str = "items";

type Rocks = raisin_rocksdb::RocksDBStorage;

/// (kind, changed_properties) of every node event.
#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<(NodeEventKind, Option<Vec<String>>)>>>);

impl EventHandler for Recorder {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        if let Event::Node(ne) = event {
            let changed = ne
                .metadata
                .as_ref()
                .and_then(|m| m.get("changed_properties"))
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            self.0.lock().unwrap().push((ne.kind.clone(), changed));
        }
        Box::pin(async { Ok(()) })
    }
    fn name(&self) -> &str {
        "changed-properties-recorder"
    }
}

impl Recorder {
    fn updated_naming(&self, property: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, changed)| {
                matches!(kind, NodeEventKind::Updated)
                    && changed
                        .as_ref()
                        .is_some_and(|c| c.iter().any(|p| p == property))
            })
            .count()
    }
}

async fn setup() -> (Arc<Rocks>, TempDir, Recorder) {
    let dir = TempDir::new().expect("temp dir");
    let storage = Rocks::new(dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Order" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    let recorder = Recorder::default();
    storage.event_bus().subscribe(Arc::new(recorder.clone()));
    (Arc::new(storage), dir, recorder)
}

fn engine(storage: &Arc<Rocks>) -> QueryEngine<Rocks> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(raisin_models::auth::AuthContext::system())
}

async fn drain(mut stream: raisin_sql_execution::RowStream) {
    while let Some(row) = stream.next().await {
        row.expect("row");
    }
}

async fn run(engine: &QueryEngine<Rocks>, sql: &str) {
    drain(engine.execute(sql).await.expect(sql)).await;
}

/// The bound-parameter form functions use (`raisin_sdk::sql::execute`).
async fn merge(engine: &QueryEngine<Rocks>, patch: serde_json::Value) {
    let stream = engine
        .execute_with_params(
            "UPDATE items SET properties = properties || $1::jsonb WHERE path = $2",
            &[patch, serde_json::json!("/order")],
            &raisin_sql::format_param_value,
        )
        .await
        .expect("merge");
    drain(stream).await;
}

async fn create(storage: &Arc<Rocks>) {
    run(
        &engine(storage),
        "INSERT INTO items (path, node_type, properties) VALUES \
         ('/order', 'test:Order', '{\"status\":\"open\"}'::JSONB)",
    )
    .await;
}

/// Event delivery is asynchronous: wait until `n` matching events arrived.
async fn wait_for(recorder: &Recorder, property: &str, n: usize) -> usize {
    for _ in 0..100 {
        if recorder.updated_naming(property) >= n {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    recorder.updated_naming(property)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_status_write_right_behind_another_write_names_status() {
    let (storage, _dir, recorder) = setup().await;
    create(&storage).await;
    let e = engine(&storage);
    for round in 0..8 {
        merge(
            &e,
            serde_json::json!({ "reservations": [format!("r{round}")] }),
        )
        .await;
        merge(&e, serde_json::json!({ "status": format!("s{round}") })).await;
    }
    assert_eq!(wait_for(&recorder, "status", 8).await, 8);
    assert_eq!(wait_for(&recorder, "reservations", 8).await, 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn racing_writes_each_name_their_own_property() {
    let (storage, _dir, recorder) = setup().await;
    create(&storage).await;
    let mut tasks = Vec::new();
    for round in 0..8 {
        for (key, value) in [
            ("reservations", format!("r{round}")),
            ("status", format!("s{round}")),
        ] {
            let storage = storage.clone();
            tasks.push(tokio::spawn(async move {
                merge(&engine(&storage), serde_json::json!({ key: value })).await;
            }));
        }
    }
    for t in tasks {
        t.await.expect("join");
    }
    assert_eq!(wait_for(&recorder, "status", 8).await, 8);
    assert_eq!(wait_for(&recorder, "reservations", 8).await, 8);
}
