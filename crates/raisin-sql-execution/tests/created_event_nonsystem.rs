//! Reproduction probe for "SQL INSERT does not fire Created triggers": verify
//! that a SQL INSERT performed by a NON-SYSTEM (RLS-enforced) identity still
//! emits a `NodeEventKind::Created` on the event bus — the signal node-event
//! triggers are built on. A system INSERT is the control. If non-system INSERT
//! emitted something other than Created (or nothing), Created triggers would
//! silently never fire for app writes.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, Event, EventBus, EventHandler, NodeEventKind,
    NodeTypeRepository, RepoScope, Storage, WorkspaceRepository,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const WS: &str = "items";

/// Records (kind, path) for every NodeEvent published to the bus.
struct Recorder {
    events: Arc<Mutex<Vec<(NodeEventKind, String)>>>,
}

impl EventHandler for Recorder {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        if let Event::Node(ne) = event {
            self.events
                .lock()
                .unwrap()
                .push((ne.kind.clone(), ne.path.clone().unwrap_or_default()));
        }
        Box::pin(async { Ok(()) })
    }
    fn name(&self) -> &str {
        "test-recorder"
    }
}

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
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
            serde_json::from_value(serde_json::json!({ "name": "test:Item" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (storage, temp_dir)
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth: AuthContext,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(auth)
}

/// A non-system identity that may create/read/update anywhere (no RLS condition).
fn writer_user() -> AuthContext {
    AuthContext::for_user("writer").with_permissions(ResolvedPermissions {
        user_id: "writer".into(),
        email: Some("writer@test.com".into()),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new(
            "/**",
            vec![Operation::Create, Operation::Read, Operation::Update],
        )],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

/// Wait until a Created event for `path` is recorded (bus dispatch is async).
async fn saw_created(events: &Arc<Mutex<Vec<(NodeEventKind, String)>>>, path: &str) -> bool {
    for _ in 0..40 {
        if events
            .lock()
            .unwrap()
            .iter()
            .any(|(k, p)| *k == NodeEventKind::Created && p == path)
        {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn sql_insert_emits_created_for_system_and_nonsystem() {
    let (storage, _td) = setup().await;

    let events = Arc::new(Mutex::new(Vec::new()));
    storage
        .event_bus()
        .subscribe(Arc::new(Recorder {
            events: events.clone(),
        }));

    // Control: a system INSERT emits Created.
    run(
        &engine(&storage, AuthContext::system()),
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('sys','/sys-item','test:Item','{\"title\":\"S\"}'::JSONB)",
    )
    .await;
    assert!(
        saw_created(&events, "/sys-item").await,
        "system INSERT must emit a Created event"
    );

    // The real question: a NON-SYSTEM identity's INSERT must also emit Created.
    run(
        &engine(&storage, writer_user()),
        "INSERT INTO items (id, path, node_type, properties) VALUES \
         ('usr','/user-item','test:Item','{\"title\":\"U\"}'::JSONB)",
    )
    .await;
    assert!(
        saw_created(&events, "/user-item").await,
        "non-system INSERT must ALSO emit a Created event (else Created triggers never fire for app writes)"
    );

    // Sanity: the non-system node must not have been emitted as Updated instead.
    let recorded: Vec<_> = events.lock().unwrap().clone();
    let user_kinds: Vec<_> = recorded
        .iter()
        .filter(|(_, p)| p == "/user-item")
        .map(|(k, _)| k.clone())
        .collect();
    assert!(
        user_kinds.contains(&NodeEventKind::Created),
        "non-system /user-item events were {:?}, expected to include Created",
        user_kinds
    );
}
