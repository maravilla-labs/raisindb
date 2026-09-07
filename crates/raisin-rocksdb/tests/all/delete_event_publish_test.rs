// SPDX-License-Identifier: BSL-1.1
//
//! `node:deleted` must be published for EVERY delete path, with the pre-delete
//! node in `metadata.node_data`.
//!
//! The default delete (`DeleteNodeOptions::default()`, cascade = true) went
//! through `delete_with_cascade`, which wrote tombstones and replication
//! captures but published no `NodeEvent` at all — only the single-node
//! `delete_impl` did. WebSocket subscribers therefore never saw `node:deleted`
//! for an ordinary delete, while `node:created` / `node:updated` arrived fine.
//! Both paths now go through one emitter (`publish_deleted_event`).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raisin_context::RepositoryConfig;
use raisin_error::Result;
use raisin_events::{Event, EventHandler, NodeEvent, NodeEventKind};
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::StorageScope;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;

const TENANT: &str = "delete-event-tenant";
const REPO: &str = "delete-event-repo";
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
                if matches!(e.kind, NodeEventKind::Deleted) {
                    self.events.lock().unwrap().push(e.clone());
                }
            }
            Ok(())
        })
    }
}

async fn storage() -> Result<(TempDir, Arc<RocksDBStorage>)> {
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path())?);
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await?;
    Ok((temp_dir, storage))
}

fn make_node(path: &str) -> Node {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        children: Vec::new(),
        order_key: "a0".to_string(),
        has_children: None,
        version: 1,
        archetype: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        created_by: Some("test-user".to_string()),
        updated_by: Some("test-user".to_string()),
        published_at: None,
        published_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

fn no_validation() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

async fn wait_for(events: &Arc<Mutex<Vec<NodeEvent>>>, expected: usize) -> Vec<NodeEvent> {
    for _ in 0..200 {
        if events.lock().unwrap().len() >= expected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    events.lock().unwrap().clone()
}

fn node_data_path(event: &NodeEvent) -> Option<String> {
    event
        .metadata
        .as_ref()
        .and_then(|m| m.get("node_data"))
        .and_then(|v| serde_json::from_value::<Node>(v.clone()).ok())
        .map(|n| n.path)
}

/// The default (cascade) delete emits one `Deleted` event per removed node,
/// root and descendants alike, each carrying the pre-delete node.
#[tokio::test]
async fn cascade_delete_publishes_deleted_event_for_root_and_descendants() -> Result<()> {
    let (_dir, storage) = storage().await?;
    let nodes = storage.nodes();
    let parent = make_node("/parent");
    let parent_id = parent.id.clone();
    nodes.create(scope(), parent, no_validation()).await?;
    nodes
        .create(scope(), make_node("/parent/child"), no_validation())
        .await?;
    nodes
        .create(
            scope(),
            make_node("/parent/child/grandchild"),
            no_validation(),
        )
        .await?;

    let events = Arc::new(Mutex::new(Vec::new()));
    storage.event_bus().subscribe(Arc::new(NodeEventRecorder {
        events: events.clone(),
    }));

    assert!(
        nodes
            .delete(scope(), &parent_id, DeleteNodeOptions::default())
            .await?,
        "cascade delete must report success"
    );

    let got = wait_for(&events, 3).await;
    let mut paths: Vec<String> = got.iter().filter_map(|e| e.path.clone()).collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["/parent", "/parent/child", "/parent/child/grandchild"],
        "one node:deleted per removed node; got {got:?}"
    );

    for e in &got {
        assert_eq!(e.workspace_id, WORKSPACE);
        assert_eq!(e.node_type.as_deref(), Some("raisin:Folder"));
        assert_eq!(
            node_data_path(e),
            e.path.clone(),
            "metadata.node_data must carry the pre-delete node"
        );
    }
    Ok(())
}

/// The single-node path (cascade = false) still emits exactly one event, with
/// the same shape as the cascade path.
#[tokio::test]
async fn non_cascade_delete_publishes_one_deleted_event() -> Result<()> {
    let (_dir, storage) = storage().await?;
    let nodes = storage.nodes();
    let leaf = make_node("/leaf");
    let leaf_id = leaf.id.clone();
    nodes.create(scope(), leaf, no_validation()).await?;

    let events = Arc::new(Mutex::new(Vec::new()));
    storage.event_bus().subscribe(Arc::new(NodeEventRecorder {
        events: events.clone(),
    }));

    let options = DeleteNodeOptions {
        cascade: false,
        ..DeleteNodeOptions::default()
    };
    assert!(nodes.delete(scope(), &leaf_id, options).await?);

    let got = wait_for(&events, 1).await;
    assert_eq!(got.len(), 1, "exactly one node:deleted; got {got:?}");
    assert_eq!(got[0].node_id, leaf_id);
    assert_eq!(got[0].path.as_deref(), Some("/leaf"));
    assert_eq!(node_data_path(&got[0]).as_deref(), Some("/leaf"));
    Ok(())
}
