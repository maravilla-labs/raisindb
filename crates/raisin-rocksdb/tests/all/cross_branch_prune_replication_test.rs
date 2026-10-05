//! Plan "Phase 11d": a promotion's `delete_missing` prune was the last
//! emitter of a pre-v2 node op — one `DeleteNode { node_id }` per pruned node,
//! captured beside the copy's `ApplyRevision`. It now rides in that same
//! `ApplyRevision` as a `Delete` change carrying the pre-delete node and its
//! placement, like every other delete, so the legacy op could be removed.
//! A replica that receives the promotion — as the oplog holds it, or
//! decomposed as the push path sends it — prunes the node too.

use crate::translation_replication_test::{node, receive, Node as Peer};
use crate::translation_substrate_test::{B, R, T, WS};
use raisin_models::nodes::Node;
use raisin_replication::operation::ReplicatedNodeChangeKind;
use raisin_replication::{decompose_operation, OpType, Operation};
use raisin_rocksdb::OpLogRepository;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository, Storage, StorageScope,
};

pub(crate) const PUBLISH: &str = "publish";

pub(crate) async fn with_publish_branch(peer: &Peer) {
    peer.storage
        .branches()
        .create_branch(T, R, PUBLISH, "system", None, None, false, false)
        .await
        .unwrap();
}

pub(crate) fn folder(name: &str, path: &str) -> Node {
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name: name.to_string(),
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    }
}

pub(crate) async fn create(peer: &Peer, node: &Node) {
    let options = CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    };
    peer.storage
        .nodes()
        .create(StorageScope::new(T, R, B, WS), node.clone(), options)
        .await
        .expect("create");
}

async fn promote(peer: &Peer) {
    promote_with(peer, true).await;
}

/// Promote `/section` (recursively) from main onto publish.
pub(crate) async fn promote_with(peer: &Peer, delete_missing: bool) {
    let roots = vec!["/section".to_string()];
    peer.storage
        .nodes()
        .copy_nodes_across_branches(
            T,
            R,
            B,
            PUBLISH,
            WS,
            &roots,
            true,
            delete_missing,
            None,
            None,
        )
        .await
        .expect("promotion");
}

pub(crate) async fn on_publish(peer: &Peer, id: &str) -> Option<Node> {
    peer.storage
        .nodes()
        .get(StorageScope::new(T, R, PUBLISH, WS), id, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_promotion_prune_replicates_as_a_delete_change() {
    let origin = node("origin").await;
    with_publish_branch(&origin).await;
    let section = folder("section", "/section");
    create(&origin, &section).await;
    let mut page = folder("page", "/section/page");
    page.parent = Some("section".to_string());
    create(&origin, &page).await;
    promote(&origin).await;
    assert!(on_publish(&origin, &page.id).await.is_some());

    // The page goes on main; the next promotion prunes it from publish.
    let deleted = origin
        .storage
        .nodes()
        .delete(
            StorageScope::new(T, R, B, WS),
            &page.id,
            DeleteNodeOptions::default(),
        )
        .await;
    assert!(deleted.expect("delete"));
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    promote(&origin).await;
    assert!(on_publish(&origin, &page.id).await.is_none());

    let mut ops = OpLogRepository::new(origin.storage.db().clone())
        .get_operations_from_node(T, R, &origin.id)
        .unwrap();
    ops.sort_by_key(|op| op.op_seq);

    // The prune is a Delete change inside the promotion's ApplyRevision on
    // publish: the pre-delete node, in its workspace, under its parent.
    let prune = ops
        .iter()
        .filter(|op| op.branch == PUBLISH)
        .find_map(|op| match &op.op_type {
            OpType::ApplyRevision { node_changes, .. } => node_changes
                .iter()
                .find(|c| c.kind == ReplicatedNodeChangeKind::Delete && c.node.id == page.id),
            _ => None,
        })
        .expect("the prune rides in the promotion's ApplyRevision");
    assert_eq!(prune.node.workspace.as_deref(), Some(WS));
    assert_eq!(prune.node.path, "/section/page");
    assert_eq!(prune.parent_id.as_deref(), Some(section.id.as_str()));
    assert!(!prune.cf_order_key.is_empty(), "its ORDERED_CHILDREN label");

    // Applied as the oplog holds it, and decomposed as the push path sends it.
    let decomposed: Vec<Operation> = ops.iter().cloned().flat_map(decompose_operation).collect();
    assert!(decomposed.iter().any(|op| matches!(
        &op.op_type,
        OpType::DeleteNodeSnapshot { node_id, .. } if node_id == &page.id
    )));
    for (name, batch) in [("replica-raw", &ops), ("replica-decomposed", &decomposed)] {
        let replica = node(name).await;
        with_publish_branch(&replica).await;
        receive(&replica, batch).await;
        assert!(
            on_publish(&replica, &section.id).await.is_some(),
            "{name}: the promoted section"
        );
        assert!(
            on_publish(&replica, &page.id).await.is_none(),
            "{name}: the pruned page is still live on publish"
        );
    }
}
