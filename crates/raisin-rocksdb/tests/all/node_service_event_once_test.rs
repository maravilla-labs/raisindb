//! One NodeService write must publish ONE node event.
//!
//! `RocksDBTransaction::commit` already publishes a `NodeEvent` per changed
//! node. `NodeService::{put, create, update_node}` then published a second,
//! metadata-less copy of the same event after the commit returned — a leftover
//! from backends whose commit publishes nothing. Every subscriber therefore
//! saw each SDK write twice. The job handler's idempotent enqueue hid it while
//! both copies arrived within microseconds, but whenever the second was
//! delayed past the first job's completion (e.g. behind a 650 ms trigger
//! registry reload) it enqueued a second TriggerEvaluation, and a flow keyed
//! on the write ran twice for the same subject_version.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raisin_context::RepositoryConfig;
use raisin_core::services::node_service::NodeService;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_events::{Event, EventHandler, NodeEvent, NodeEventKind};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "event-once-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

struct NodeEventRecorder {
    events: Arc<Mutex<Vec<NodeEvent>>>,
}

impl EventHandler for NodeEventRecorder {
    fn name(&self) -> &str {
        "node_event_recorder"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Node(e) = event {
                self.events.lock().unwrap().push(e.clone());
            }
            Ok(())
        })
    }
}

fn build_node(path: &str, title: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("user".to_string()),
        created_by: Some("user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

async fn setup(temp_dir: &TempDir) -> Result<Arc<RocksDBStorage>> {
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let repo_config = RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: Some("Branch head monotonicity test".to_string()),
        tags: HashMap::new(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    // Seed the node type BEFORE creating the workspace: WorkspaceService::put
    // bootstraps a ROOT node for new workspaces, which requires the default
    // folder type to already be registered.
    let folder_type = NodeType {
        id: Some("raisin:Folder".to_string()),
        strict: Some(false),
        name: "raisin:Folder".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            folder_type,
            CommitMetadata::system("seed folder type"),
        )
        .await?;

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    Ok(storage)
}

fn count(events: &Arc<Mutex<Vec<NodeEvent>>>, node_id: &str, kind: NodeEventKind) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.node_id == node_id && e.kind == kind)
        .count()
}

/// Give every spawned handler task time to run, so a late duplicate is seen.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(200)).await;
}

#[tokio::test]
async fn node_service_writes_publish_exactly_one_event_each() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let storage = setup(&temp_dir).await?;

    let events = Arc::new(Mutex::new(Vec::new()));
    storage.event_bus().subscribe(Arc::new(NodeEventRecorder {
        events: events.clone(),
    }));

    let svc = NodeService::new_with_context(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
        WORKSPACE.to_string(),
    )
    .with_auth(AuthContext::system());

    // create()
    let created = build_node("/created", "Created");
    svc.create(created.clone()).await?;
    settle().await;
    assert_eq!(
        count(&events, &created.id, NodeEventKind::Created),
        1,
        "create() must publish one Created event"
    );

    // update_node() — the SDK `nodes().update` path from the incident.
    let mut updated = created.clone();
    updated.properties.insert(
        "title".to_string(),
        PropertyValue::String("Renamed".to_string()),
    );
    svc.update_node(updated).await?;
    settle().await;
    assert_eq!(
        count(&events, &created.id, NodeEventKind::Updated),
        1,
        "update_node() must publish one Updated event"
    );

    // put() of a new node.
    let put = build_node("/put", "Put");
    svc.put(put.clone()).await?;
    settle().await;
    assert_eq!(
        count(&events, &put.id, NodeEventKind::Created),
        1,
        "put() of a new node must publish one Created event"
    );

    // The one event left is the commit's, which carries the data downstream
    // consumers read instead of going back to storage.
    let with_data = events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.node_id == created.id && e.kind == NodeEventKind::Updated)
        .all(|e| {
            e.metadata
                .as_ref()
                .is_some_and(|m| m.contains_key("node_data"))
        });
    assert!(
        with_data,
        "the surviving event must be the commit's, with node_data"
    );

    Ok(())
}
