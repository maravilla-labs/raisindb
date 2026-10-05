//! The ONE writer of a node RECORD: its `NODES` blob and its `NODE_PATH` entry.
//!
//! Every write path that stages a live node blob comes through here (plan
//! Phases 10 and 10b):
//!
//! - the repository create / update / move / reorder path
//!   (`add_node_indexes_to_batch_with_parent_id`, `rewrite_node_record_to_batch`);
//! - the batch path behind copy, promotion and deep create
//!   (`add_node_to_batch_with_parent_id`);
//! - the transaction path behind SQL DML and the WebSocket handlers
//!   (`put_node`, `add_node`, `move_node_tree`);
//! - branch merge apply (`branches/merge/apply.rs`);
//! - the replication applicator (`applicator/crdt_ops.rs`);
//! - backup import (`management/backup/import.rs`).
//!
//! There is ONE format: a `StorageNode` blob (no path) and a `NODE_PATH`
//! entry. Before Phase 10 the transaction path, merge and the replication
//! applicators stored the full `Node`, path embedded — and the transaction
//! path no `NODE_PATH` entry at all. Such blobs stay on disk in existing
//! databases and are read forever by the path read rule
//! (`crate::mvcc_read::materialize_path`); the `node_path` backfill gives them
//! the entries their writers never wrote. Nothing writes them any more, so a
//! binary older than the read rule cannot open a database this one has
//! written to (downgrade unsupported, plan Phase 10b).
//!
//! Not part of the record: PATH_INDEX (path → id), whose writers differ in
//! what they tombstone, and the virtual-mount registry.
//!
//! # A record always asserts its path
//!
//! [`write_node_record`] writes `NODE_PATH` at the record's own revision.
//! The path-less "keep the current path" write existed only for replication's
//! pre-v2 property and move ops (`set_property`, `move_node`); with those ops
//! gone (plan "Phase 11d") every writer knows where its node is.

use super::super::super::storage_node::StorageNode;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

/// Where a record is written: `(tenant, repo, branch, workspace)`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecordScope<'a> {
    pub(crate) tenant_id: &'a str,
    pub(crate) repo_id: &'a str,
    pub(crate) branch: &'a str,
    pub(crate) workspace: &'a str,
}

impl<'a> RecordScope<'a> {
    pub(crate) fn new(
        tenant_id: &'a str,
        repo_id: &'a str,
        branch: &'a str,
        workspace: &'a str,
    ) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace,
        }
    }

    fn names(&self) -> crate::localized_name::keys::NameScope<'a> {
        crate::localized_name::keys::NameScope::new(
            self.tenant_id,
            self.repo_id,
            self.branch,
            self.workspace,
        )
    }
}

/// Stage `node`'s record at `revision` in `batch` — the blob, and `NODE_PATH`
/// naming `node.path` at the same revision. Returns the `NODES` key.
///
/// `parent_id` is stored in the blob for parent resolution without path
/// parsing; `None` for a root child (callers holding the ORDERED_CHILDREN
/// root key `"/"` pass it through [`parent_id_of`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_node_record(
    db: &DB,
    batch: &mut WriteBatch,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    parent_id: Option<String>,
    revision: &HLC,
) -> Result<Vec<u8>> {
    let scope = RecordScope::new(tenant_id, repo_id, branch, workspace);
    put_node_path(db, batch, scope, &node.id, revision, &node.path)?;
    // The localized name index rides on the record (plan Phase 12): every
    // writer of a live node comes through here.
    crate::localized_name::sync::sync_node(
        db,
        batch,
        scope.names(),
        node,
        parent_id.as_deref(),
        revision,
    )?;
    put_blob(db, batch, scope, node, parent_id, revision)
}

/// `parent_id` as a record stores it: the ORDERED_CHILDREN root key `"/"`
/// (and an empty id) mean "no parent".
pub(crate) fn parent_id_of(parent_key: Option<&str>) -> Option<String> {
    parent_key
        .filter(|p| !p.is_empty() && *p != "/")
        .map(str::to_string)
}

/// The blob: a `StorageNode`, map-encoded (`to_vec_named`) so nested types
/// such as `RaisinReference` serialize with their field names.
fn put_blob(
    db: &DB,
    batch: &mut WriteBatch,
    scope: RecordScope<'_>,
    node: &Node,
    parent_id: Option<String>,
    revision: &HLC,
) -> Result<Vec<u8>> {
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let node_key = keys::node_key_versioned(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        &node.id,
        revision,
    );
    let node_value = rmp_serde::to_vec_named(&StorageNode::from_node(node, parent_id))
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;
    batch.put_cf(cf_nodes, node_key.clone(), node_value);
    Ok(node_key)
}

/// `node_id -> path` at `revision`: O(1) path materialization.
fn put_node_path(
    db: &DB,
    batch: &mut WriteBatch,
    scope: RecordScope<'_>,
    node_id: &str,
    revision: &HLC,
    path: &str,
) -> Result<()> {
    let cf_node_path = cf_handle(db, cf::NODE_PATH)?;
    let key = keys::node_path_key_versioned(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        node_id,
        revision,
    );
    batch.put_cf(cf_node_path, key, path.as_bytes());
    Ok(())
}
