use std::collections::HashMap;
use std::sync::Arc;

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, RelationRef};
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    OpType, Operation, VectorClock,
};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::StorageScope;
use raisin_storage::{
    BranchRepository, NodeRepository, RelationRepository, Storage, TranslationRepository,
};
use tempfile::TempDir;
use uuid::Uuid;

async fn seed_branch(storage: &Arc<RocksDBStorage>, tenant: &str, repo: &str, branch: &str) {
    let _ = storage
        .branches()
        .create_branch(tenant, repo, branch, "system", None, None, false, false)
        .await;
}

fn cf_key(node: &Node) -> String {
    format!("{}::{}", node.order_key, node.id)
}

#[tokio::test]
async fn apply_revision_replays_full_node_state() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;

    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch_name = "main";
    let workspace = "default";

    seed_branch(&storage, tenant_id, repo_id, branch_name).await;

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String("Hello World".to_string()),
    );

    let node = Node {
        id: "article-1".to_string(),
        name: "Hello World".to_string(),
        path: "/hello-world".to_string(),
        node_type: "Article".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: "a".to_string(),
        has_children: None,
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("user1".to_string()),
        created_by: Some("user1".to_string()),
        translations: None,
        tenant_id: Some(tenant_id.to_string()),
        workspace: Some(workspace.to_string()),
        owner_id: None,
        relations: Vec::new(),
    };

    let revision = HLC::new(10, 0);
    let op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch_name.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision.clone(),
            node_changes: vec![ReplicatedNodeChange {
                node: node.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: cf_key(&node),
            }],
        },
        revision: Some(revision.clone()),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        acknowledged_by: Default::default(),
    };

    applicator
        .apply_operation(&op)
        .await
        .expect("apply revision");

    let node_repo = storage.nodes();
    let stored = node_repo
        .get(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &node.id,
            None,
        )
        .await
        .unwrap()
        .expect("node must exist");
    assert_eq!(stored.path, node.path);
    assert_eq!(
        stored.properties.get("title"),
        Some(&PropertyValue::String("Hello World".to_string()))
    );

    let fetched_by_path = node_repo
        .get_by_path(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &node.path,
            None,
        )
        .await
        .unwrap()
        .expect("path lookup");
    assert_eq!(fetched_by_path.id, node.id);
}

#[tokio::test]
async fn apply_delete_node_removes_translations() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;

    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch_name = "main";
    let workspace = "default";

    seed_branch(&storage, tenant_id, repo_id, branch_name).await;

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let mut node = Node {
        id: Uuid::new_v4().to_string(),
        name: "Hello World".to_string(),
        path: "/hello-world".to_string(),
        node_type: "Article".to_string(),
        archetype: None,
        properties: {
            let mut map = HashMap::new();
            map.insert(
                "title".to_string(),
                PropertyValue::String("Hello World".to_string()),
            );
            map
        },
        children: Vec::new(),
        order_key: "a".to_string(),
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
        tenant_id: Some(tenant_id.to_string()),
        workspace: Some(workspace.to_string()),
        owner_id: None,
        relations: Vec::new(),
    };

    let revision = HLC::new(10, 0);
    let create_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch_name.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision.clone(),
            node_changes: vec![ReplicatedNodeChange {
                node: node.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: cf_key(&node),
            }],
        },
        revision: Some(revision.clone()),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        acknowledged_by: Default::default(),
    };

    applicator
        .apply_operation(&create_op)
        .await
        .expect("create apply");

    let locale = LocaleCode::parse("de").expect("valid locale");
    let mut overlay_map = HashMap::new();
    overlay_map.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Hallo Welt".to_string()),
    );
    let overlay = LocaleOverlay::properties(overlay_map);
    let translation_revision = HLC::new(11, 0);
    let translation_meta = TranslationMeta::system(
        locale.clone(),
        translation_revision.clone(),
        "init".to_string(),
    );

    storage
        .translations()
        .store_translation(
            tenant_id,
            repo_id,
            branch_name,
            workspace,
            &node.id,
            &locale,
            &overlay,
            &translation_meta,
        )
        .await
        .expect("store translation");

    assert!(storage
        .translations()
        .get_translation(
            tenant_id,
            repo_id,
            branch_name,
            workspace,
            &node.id,
            &locale,
            &translation_revision
        )
        .await
        .expect("get translation")
        .is_some());

    let delete_revision = HLC::new(20, 0);
    let delete_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 2,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch_name.to_string(),
        op_type: OpType::DeleteNode {
            node_id: node.id.clone(),
        },
        revision: Some(delete_revision.clone()),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        acknowledged_by: Default::default(),
    };

    applicator
        .apply_operation(&delete_op)
        .await
        .expect("delete apply");

    assert!(storage
        .translations()
        .get_translation(
            tenant_id,
            repo_id,
            branch_name,
            workspace,
            &node.id,
            &locale,
            &delete_revision
        )
        .await
        .expect("get translation post delete")
        .is_none());
}

#[tokio::test]
async fn apply_revision_delete_removes_relations_and_translations() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;

    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());

    let tenant_id = "tenant1";
    let repo_id = "repo1";
    let branch_name = "main";
    let workspace = "default";

    seed_branch(&storage, tenant_id, repo_id, branch_name).await;

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let mut base_node = |id: &str, name: &str, path: &str| Node {
        id: id.to_string(),
        name: name.to_string(),
        path: path.to_string(),
        node_type: "Article".to_string(),
        archetype: None,
        properties: {
            let mut map = HashMap::new();
            map.insert("title".to_string(), PropertyValue::String(name.to_string()));
            map
        },
        children: Vec::new(),
        order_key: "a".to_string(),
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
        tenant_id: Some(tenant_id.to_string()),
        workspace: Some(workspace.to_string()),
        owner_id: None,
        relations: Vec::new(),
    };

    let mut source_node = base_node("source-node", "Source", "/source");
    let mut target_node = base_node("target-node", "Target", "/target");

    let create_revision = HLC::new(5, 0);
    let create_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch_name.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: create_revision.clone(),
            node_changes: vec![
                ReplicatedNodeChange {
                    node: source_node.clone(),
                    parent_id: Some("/".to_string()),
                    kind: ReplicatedNodeChangeKind::Upsert,
                    cf_order_key: cf_key(&source_node),
                },
                ReplicatedNodeChange {
                    node: target_node.clone(),
                    parent_id: Some("/".to_string()),
                    kind: ReplicatedNodeChangeKind::Upsert,
                    cf_order_key: cf_key(&target_node),
                },
            ],
        },
        revision: Some(create_revision.clone()),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        acknowledged_by: Default::default(),
    };

    applicator
        .apply_operation(&create_op)
        .await
        .expect("create nodes via ApplyRevision");

    let relation = RelationRef::simple(
        target_node.id.clone(),
        workspace.to_string(),
        target_node.node_type.clone(),
        "references".to_string(),
    );
    storage
        .relations()
        .add_relation(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &source_node.id,
            &source_node.node_type,
            relation,
        )
        .await
        .expect("add relation");

    let locale = LocaleCode::parse("fr").expect("valid locale");
    let mut overlay_map = HashMap::new();
    overlay_map.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Bonjour".to_string()),
    );
    let translation_revision = HLC::new(7, 0);
    let overlay = LocaleOverlay::properties(overlay_map);
    let translation_meta = TranslationMeta::system(
        locale.clone(),
        translation_revision.clone(),
        "init".to_string(),
    );
    storage
        .translations()
        .store_translation(
            tenant_id,
            repo_id,
            branch_name,
            workspace,
            &source_node.id,
            &locale,
            &overlay,
            &translation_meta,
        )
        .await
        .expect("store translation");

    assert!(
        storage
            .relations()
            .get_outgoing_relations(
                StorageScope::new(tenant_id, repo_id, branch_name, workspace),
                &source_node.id,
                None
            )
            .await
            .expect("fetch outgoing")
            .len()
            == 1
    );

    let stored_node = storage
        .nodes()
        .get(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &source_node.id,
            None,
        )
        .await
        .expect("read node")
        .expect("node exists");

    let delete_revision = HLC::new(20, 0);
    let delete_op = Operation {
        op_id: Uuid::new_v4(),
        op_seq: 3,
        cluster_node_id: "node-alpha".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch_name.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: delete_revision.clone(),
            node_changes: vec![ReplicatedNodeChange {
                node: stored_node.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Delete,
                cf_order_key: cf_key(&stored_node),
            }],
        },
        revision: Some(delete_revision.clone()),
        actor: "system".to_string(),
        message: None,
        is_system: true,
        acknowledged_by: Default::default(),
    };

    applicator
        .apply_operation(&delete_op)
        .await
        .expect("apply delete via ApplyRevision");

    assert!(storage
        .nodes()
        .get(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &source_node.id,
            None
        )
        .await
        .expect("fetch after delete")
        .is_none());

    assert!(storage
        .relations()
        .get_outgoing_relations(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &source_node.id,
            None
        )
        .await
        .expect("fetch outgoing after delete")
        .is_empty());

    assert!(storage
        .relations()
        .get_incoming_relations(
            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
            &target_node.id,
            None
        )
        .await
        .expect("fetch incoming after delete")
        .is_empty());

    assert!(storage
        .translations()
        .get_translation(
            tenant_id,
            repo_id,
            branch_name,
            workspace,
            &source_node.id,
            &locale,
            &delete_revision
        )
        .await
        .expect("translation lookup after delete")
        .is_none());
}
