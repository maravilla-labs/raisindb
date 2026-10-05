//! "Some node write on the wire" for the cluster and replication tests.
//!
//! Every node write replicates as a snapshot: `ApplyRevision`, decomposed into
//! `UpsertNodeSnapshot` before it is sent. These tests used the pre-v2
//! `capture_create_node` builder only to put a node op into the oplog; it is
//! gone with the granular node ops (plan "Phase 11d"), so they capture an
//! `UpsertNodeSnapshot` instead — an op the push path forwards as is (only an
//! `ApplyRevision` is decomposed, which would give the peer different op ids).

use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{OpType, Operation};
use raisin_rocksdb::OperationCapture;
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh, increasing revision per captured snapshot.
fn next_revision() -> HLC {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = chrono::Utc::now().timestamp_millis() as u64;
    let ms = LAST
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
            Some(last.max(now - 1) + 1)
        })
        .map(|last| last.max(now - 1) + 1)
        .unwrap();
    HLC::new(ms, 0)
}

/// The snapshot of a node with `properties` (a JSON object), as a commit
/// replicates it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn node_snapshot(
    node_id: &str,
    name: &str,
    node_type: &str,
    archetype: Option<String>,
    parent_id: Option<String>,
    order_key: &str,
    properties: serde_json::Value,
    owner_id: Option<String>,
    workspace: Option<String>,
    path: &str,
) -> OpType {
    let properties = properties
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
                .collect()
        })
        .unwrap_or_default();
    OpType::UpsertNodeSnapshot {
        node: Node {
            id: node_id.to_string(),
            name: name.to_string(),
            path: path.to_string(),
            node_type: node_type.to_string(),
            archetype,
            order_key: order_key.to_string(),
            owner_id,
            workspace: Some(workspace.unwrap_or_else(|| "default".to_string())),
            properties,
            ..Node::default()
        },
        parent_id: Some(parent_id.unwrap_or_else(|| "/".to_string())),
        revision: next_revision(),
        cf_order_key: format!("{order_key}::{node_id}"),
    }
}

/// `capture_node_snapshot` on the capture handle the tests already hold.
#[allow(async_fn_in_trait)]
pub(crate) trait CaptureNodeSnapshot {
    #[allow(clippy::too_many_arguments)]
    async fn capture_node_snapshot(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        name: String,
        node_type: String,
        archetype: Option<String>,
        parent_id: Option<String>,
        order_key: String,
        properties: serde_json::Value,
        owner_id: Option<String>,
        workspace: Option<String>,
        path: String,
        actor: String,
    ) -> raisin_error::Result<Operation>;
}

impl CaptureNodeSnapshot for OperationCapture {
    async fn capture_node_snapshot(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        name: String,
        node_type: String,
        archetype: Option<String>,
        parent_id: Option<String>,
        order_key: String,
        properties: serde_json::Value,
        owner_id: Option<String>,
        workspace: Option<String>,
        path: String,
        actor: String,
    ) -> raisin_error::Result<Operation> {
        let op_type = node_snapshot(
            &node_id, &name, &node_type, archetype, parent_id, &order_key, properties, owner_id,
            workspace, &path,
        );
        self.capture_operation(tenant_id, repo_id, branch, op_type, actor, None, false)
            .await
    }
}
