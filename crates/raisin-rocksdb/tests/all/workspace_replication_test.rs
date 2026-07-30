// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Workspace records must both replicate and resolve conflicts.
//!
//! `OpType::UpdateWorkspace` shipped with an applier and **no producer**: nothing
//! in the tree ever emitted one, so a workspace record — and with it every
//! workspace config, including the spatial index policy — was silently local to
//! the node it was written on. The applier, meanwhile, was a blind overwrite, so
//! simply adding a producer would have let an out-of-order or replayed peer
//! message carrying an OLDER workspace revert a newer config.
//!
//! Both halves are asserted here against a real storage instance and the real
//! applier, rather than against the pure decision function alone (which is unit
//! tested next to itself in `replication/application/applicator/workspace_lww.rs`):
//! only this level proves the producer is actually wired into the write path and
//! that the applier consults the stored record before overwriting it.

use std::sync::Arc;

use raisin_error::Result;
use raisin_models::nodes::types::initial_structure::{InitialChild, InitialNodeStructure};
use raisin_models::timestamp::StorageTimestamp;
use raisin_models::workspace::{Workspace, WorkspaceConfig};
use raisin_replication::{OpType, Operation};
use raisin_rocksdb::replication::OperationApplicator;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::RepoScope;
use raisin_storage::{Storage, WorkspaceRepository};
use tempfile::TempDir;

const TENANT: &str = "ws-repl-tenant";
const REPO: &str = "ws-repl-repo";
const WS: &str = "places";

fn storage_with_replication(node_id: &str) -> (TempDir, Arc<RocksDBStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;
    config.cluster_node_id = Some(node_id.to_string());
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    (temp_dir, storage)
}

fn applicator(storage: &Arc<RocksDBStorage>) -> OperationApplicator {
    OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    )
}

/// A workspace whose description carries `marker`, so the two sides of a conflict
/// stay distinguishable after a write.
fn workspace(marker: &str) -> Workspace {
    let mut ws = Workspace::new(WS.to_string());
    ws.description = Some(marker.to_string());
    ws.allowed_node_types = vec!["raisin:Folder".to_string()];
    ws.allowed_root_node_types = vec!["raisin:Folder".to_string()];
    ws.config = WorkspaceConfig::default();
    ws
}

fn scope() -> RepoScope<'static> {
    RepoScope::new(TENANT, REPO)
}

async fn stored(storage: &Arc<RocksDBStorage>) -> Result<Option<Workspace>> {
    storage.workspaces().get(scope(), WS).await
}

/// Wrap a workspace in the operation a peer would have sent for it.
fn update_workspace_op(cluster_node_id: &str, ws: &Workspace) -> Operation {
    Operation {
        op_id: uuid::Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: cluster_node_id.to_string(),
        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        vector_clock: raisin_replication::VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: "main".to_string(),
        op_type: OpType::UpdateWorkspace {
            workspace_id: WS.to_string(),
            workspace: ws.clone(),
        },
        revision: None,
        actor: "peer".to_string(),
        agent: None,
        message: None,
        is_system: false,
        acknowledged_by: Default::default(),
    }
}

/// THE producer gap: writing a workspace must put an `UpdateWorkspace` in the
/// operation log, or nothing is ever sent to a peer.
#[tokio::test]
async fn workspace_write_is_captured_for_replication() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");

    storage
        .workspaces()
        .put(scope(), workspace("first"))
        .await?;

    let oplog = OpLogRepository::new(storage.db().clone());
    let by_node = oplog.get_all_operations(TENANT, REPO)?;
    let captured: Vec<&Operation> = by_node
        .values()
        .flatten()
        .filter(|op| matches!(op.op_type, OpType::UpdateWorkspace { .. }))
        .collect();

    assert_eq!(
        captured.len(),
        1,
        "expected exactly one UpdateWorkspace in the oplog, found {} (all ops: {:?})",
        captured.len(),
        by_node
            .values()
            .flatten()
            .map(|op| format!("{:?}", op.op_type))
            .collect::<Vec<_>>()
    );

    let OpType::UpdateWorkspace {
        workspace_id,
        workspace,
    } = &captured[0].op_type
    else {
        unreachable!("filtered above")
    };
    assert_eq!(workspace_id, WS);
    assert_eq!(workspace.description.as_deref(), Some("first"));
    Ok(())
}

/// A config change must be captured too, carrying the changed config — this is
/// the spatial-policy fan-out path.
#[tokio::test]
async fn workspace_config_change_is_captured_with_the_new_config() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");

    storage
        .workspaces()
        .put(scope(), workspace("created"))
        .await?;

    let mut updated = stored(&storage).await?.expect("workspace must exist");
    updated.config.default_branch = "release".to_string();
    storage.workspaces().put(scope(), updated).await?;

    let oplog = OpLogRepository::new(storage.db().clone());
    let by_node = oplog.get_all_operations(TENANT, REPO)?;
    let configs: Vec<String> = by_node
        .values()
        .flatten()
        .filter_map(|op| match &op.op_type {
            OpType::UpdateWorkspace { workspace, .. } => {
                Some(workspace.config.default_branch.clone())
            }
            _ => None,
        })
        .collect();

    assert!(
        configs.contains(&"release".to_string()),
        "the changed config never reached the oplog; captured configs: {configs:?}"
    );
    Ok(())
}

/// The write path must stamp `updated_at`, because that is the only comparator
/// the applier has. An unstamped record is unorderable on its peers.
#[tokio::test]
async fn update_stamps_updated_at_and_preserves_created_at() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");

    storage
        .workspaces()
        .put(scope(), workspace("created"))
        .await?;
    let created = stored(&storage).await?.expect("workspace must exist");
    assert!(
        created.updated_at.is_none(),
        "a freshly created workspace should not claim to have been updated"
    );

    let mut edited = created.clone();
    edited.description = Some("edited".to_string());
    // A client trying to rewrite history must not win.
    edited.created_at = StorageTimestamp::from_nanos(0).unwrap();
    storage.workspaces().put(scope(), edited).await?;

    let after = stored(&storage).await?.expect("workspace must exist");
    assert_eq!(
        after.created_at, created.created_at,
        "created_at was rewritten"
    );
    assert!(
        after.updated_at.is_some(),
        "updated_at was not stamped on update, leaving the record unorderable"
    );
    Ok(())
}

/// THE regression the LWW guard exists for, through the real applier: an older
/// replicated workspace must not revert a newer stored one.
#[tokio::test]
async fn older_replicated_workspace_does_not_clobber_newer_local() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");
    let applicator = applicator(&storage);

    // Local state: created, then updated — so it carries a recent `updated_at`.
    storage
        .workspaces()
        .put(scope(), workspace("local"))
        .await?;
    let mut newer = stored(&storage).await?.expect("workspace must exist");
    newer.description = Some("newer-local".to_string());
    storage.workspaces().put(scope(), newer).await?;
    let newer = stored(&storage).await?.expect("workspace must exist");

    // A peer message that was produced BEFORE the local update and arrives after.
    let mut older = newer.clone();
    older.description = Some("older-peer".to_string());
    older.updated_at = Some(
        StorageTimestamp::from_nanos(newer.updated_at.unwrap().timestamp_nanos() - 1_000_000_000)
            .unwrap(),
    );

    applicator
        .apply_operation(&update_workspace_op("node-b", &older))
        .await?;

    let after = stored(&storage).await?.expect("workspace must exist");
    assert_eq!(
        after.description.as_deref(),
        Some("newer-local"),
        "an older replicated workspace reverted a newer local one"
    );
    Ok(())
}

/// ...and the guard must not become a wall: a genuinely newer record still wins.
#[tokio::test]
async fn newer_replicated_workspace_applies() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");
    let applicator = applicator(&storage);

    storage
        .workspaces()
        .put(scope(), workspace("local"))
        .await?;
    let local = stored(&storage).await?.expect("workspace must exist");

    let mut newer = local.clone();
    newer.description = Some("newer-peer".to_string());
    newer.config.default_branch = "from-peer".to_string();
    newer.updated_at = Some(StorageTimestamp::now());

    applicator
        .apply_operation(&update_workspace_op("node-b", &newer))
        .await?;

    let after = stored(&storage).await?.expect("workspace must exist");
    assert_eq!(after.description.as_deref(), Some("newer-peer"));
    assert_eq!(
        after.config.default_branch, "from-peer",
        "the replicated config did not land"
    );
    Ok(())
}

/// An applied workspace must be READABLE afterwards.
///
/// This is a regression test for a bug that only became reachable once workspaces
/// started replicating: the applier wrote the record with the compact
/// (array-encoded) MessagePack helper, while `InitialNodeStructure` /
/// `InitialChild` carry hand-written deserializers that accept only NAMED fields.
/// A replicated workspace with an `initial_structure` therefore came back as
/// "InitialChild cannot be null" — and since `WorkspaceRepository::list`
/// deserializes every workspace in the repo, that one poisoned record made every
/// SQL statement in the repo fail with a 500. Found by the three-node end-to-end
/// test, which could not INSERT on a node that had received a workspace.
///
/// `list` is asserted alongside `get` precisely because that is the blast radius:
/// `get` only touches the one bad key, `list` touches all of them.
#[tokio::test]
async fn applied_workspace_with_initial_structure_is_readable() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");
    let applicator = applicator(&storage);

    let mut incoming = workspace("from-peer");
    incoming.initial_structure = Some(InitialNodeStructure {
        properties: None,
        children: Some(vec![InitialChild {
            name: "Users".to_string(),
            node_type: "raisin:Folder".to_string(),
            archetype: None,
            properties: None,
            translations: None,
            children: None,
        }]),
    });

    applicator
        .apply_operation(&update_workspace_op("node-b", &incoming))
        .await?;

    let after = stored(&storage)
        .await?
        .expect("the applied workspace must be readable");
    let children = after
        .initial_structure
        .as_ref()
        .and_then(|s| s.children.as_ref())
        .expect("initial_structure children must survive the round trip");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "Users");

    let listed = storage.workspaces().list(scope()).await?;
    assert_eq!(
        listed.len(),
        1,
        "listing the repo's workspaces must not fail on an applied record"
    );
    Ok(())
}

/// A workspace that does not exist locally is never a conflict — first sight
/// applies, which is how a workspace created on one node reaches a fresh peer.
#[tokio::test]
async fn unknown_workspace_is_created_from_the_replicated_record() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");
    let applicator = applicator(&storage);

    assert!(stored(&storage).await?.is_none());

    applicator
        .apply_operation(&update_workspace_op("node-b", &workspace("from-peer")))
        .await?;

    let after = stored(&storage)
        .await?
        .expect("workspace must have been created");
    assert_eq!(after.description.as_deref(), Some("from-peer"));
    Ok(())
}

/// Redelivering the identical operation must stay a no-op rather than being
/// rejected — replication redelivers, and idempotency is the contract.
#[tokio::test]
async fn redelivering_the_same_operation_is_idempotent() -> Result<()> {
    let (_dir, storage) = storage_with_replication("node-a");
    let applicator = applicator(&storage);

    let mut incoming = workspace("from-peer");
    incoming.updated_at = Some(StorageTimestamp::now());
    let op = update_workspace_op("node-b", &incoming);

    applicator.apply_operation(&op).await?;
    applicator.apply_operation(&op).await?;

    let after = stored(&storage).await?.expect("workspace must exist");
    assert_eq!(after.description.as_deref(), Some("from-peer"));
    Ok(())
}
