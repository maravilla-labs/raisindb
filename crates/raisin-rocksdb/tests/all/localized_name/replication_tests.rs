//! Nothing derived replicates: a replica maintains its own localized name
//! index from the replicated records (node snapshots, overlay versions, the
//! repository record), through the PRODUCTION receive path.

use super::support::*;
use raisin_replication::OperationLogStorage;
use raisin_replication::{OpType, Operation};
use raisin_rocksdb::localized_name::Availability;
use raisin_rocksdb::replication::RocksDbOperationLogStorage;
use raisin_rocksdb::{OpLogRepository, RocksDBConfig, RocksDBStorage};
use raisin_storage::localized::LocalizedServedBy::{Fallback, Index};
use raisin_storage::{DeleteNodeOptions, NodeRepository, Storage};
use std::sync::Arc;
use tempfile::TempDir;

struct Peer {
    storage: Arc<RocksDBStorage>,
    id: &'static str,
    _dir: TempDir,
}

async fn peer(id: &'static str) -> Peer {
    let dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    config.replication_enabled = true;
    config.async_operation_queue = false;
    config.cluster_node_id = Some(id.to_string());
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    provision(&storage).await;
    Peer {
        storage,
        id,
        _dir: dir,
    }
}

fn highest(origin: &Peer) -> u64 {
    OpLogRepository::new(origin.storage.db().clone())
        .get_highest_seq(T, R, origin.id)
        .unwrap()
}

fn ops_after(origin: &Peer, after: u64) -> Vec<Operation> {
    let mut ops: Vec<Operation> = OpLogRepository::new(origin.storage.db().clone())
        .get_operations_from_node(T, R, origin.id)
        .unwrap()
        .into_iter()
        .filter(|op| op.op_seq > after)
        .collect();
    ops.sort_by_key(|op| op.op_seq);
    ops
}

async fn receive(replica: &Peer, ops: &[Operation]) {
    RocksDbOperationLogStorage::new(replica.storage.clone())
        .put_operations_batch(ops)
        .await
        .unwrap();
}

/// Origin and replica with the same repository; returns the origin's
/// oplog position after provisioning.
async fn pair() -> (Peer, Peer, u64) {
    let origin = peer("origin").await;
    let replica = peer("replica").await;
    let start = highest(&origin);
    (origin, replica, start)
}

#[tokio::test]
async fn rebuilt_on_replica() {
    let (origin, replica, start) = pair().await;
    let (products, chair) = catalog(&origin.storage).await;
    receive(&replica, &ops_after(&origin, start)).await;
    let rs = &replica.storage;
    // Maintained inline by the apply path, never replicated as entries.
    assert_eq!(
        claimants(rs, B, "fr", &products, "chaise"),
        vec![chair.clone()]
    );
    assert_eq!(availability(rs, B), Availability::NotBuilt);
    assert_eq!(
        id_via(rs, B, "fr", "/produits/chaise", Fallback),
        Some(chair.clone())
    );
    let report = build(rs, B).await;
    assert_eq!(
        report.localized_names.rewritten, 0,
        "the apply path already wrote it all"
    );
    assert_eq!(id_via(rs, B, "fr", "/produits/chaise", Index), Some(chair));
}

#[tokio::test]
async fn removed_on_replicated_delete() {
    let (origin, replica, start) = pair().await;
    let (products, chair) = catalog(&origin.storage).await;
    let shipped = highest(&origin);
    receive(&replica, &ops_after(&origin, start)).await;
    build(&replica.storage, B).await;
    assert!(resolve(&replica.storage, "fr", "/produits/chaise").is_some());

    origin
        .storage
        .nodes()
        .delete(scope(B), &chair, DeleteNodeOptions::default())
        .await
        .unwrap();
    receive(&replica, &ops_after(&origin, shipped)).await;
    assert_eq!(resolve(&replica.storage, "fr", "/produits/chaise"), None);
    assert!(claimants(&replica.storage, B, "fr", &products, "chaise").is_empty());
}

#[tokio::test]
async fn default_language_change_replicated_from_peer_falls_back() {
    let (origin, replica, start) = pair().await;
    let products = create(&origin.storage, "/products", &[]).await;
    let chair = create(&origin.storage, "/products/chair", &[]).await;
    set_name(&origin.storage, &products, "de", "produkte").await;
    set_name(&origin.storage, &chair, "de", "stuhl").await;
    let shipped = highest(&origin);
    receive(&replica, &ops_after(&origin, start)).await;
    build(&replica.storage, B).await;
    assert!(availability(&replica.storage, B).is_ready());

    let mut config = repo_config();
    config.set_default_language("de");
    set_config(&origin.storage, config).await;
    let ops = ops_after(&origin, shipped);
    assert!(
        ops.iter()
            .any(|op| matches!(op.op_type, OpType::UpdateRepository { .. })),
        "the config change replicates"
    );
    receive(&replica, &ops).await;
    let rs = &replica.storage;
    // The replicated change flipped the replica's state in its apply batch:
    // no stale Ready, and the fallback serves the NEW default at once.
    assert_eq!(availability(rs, B), Availability::NotBuilt);
    assert_eq!(
        resolve(rs, "de", "/produkte/stuhl"),
        None,
        "de is canonical now"
    );
    assert_eq!(
        id_via(rs, B, "en", "/products/chair", Fallback),
        Some(chair.clone())
    );
    build(rs, B).await;
    assert_eq!(id_via(rs, B, "en", "/products/chair", Index), Some(chair));
}

#[tokio::test]
async fn out_of_order_name_apply_leaves_no_phantom() {
    let (origin, replica, start) = pair().await;
    let (products, chair) = catalog(&origin.storage).await;
    let shipped = highest(&origin);
    receive(&replica, &ops_after(&origin, start)).await;
    build(&replica.storage, B).await;

    set_name(&origin.storage, &chair, "fr", "a").await;
    set_name(&origin.storage, &chair, "fr", "b").await;
    let mut ops = ops_after(&origin, shipped);
    ops.reverse(); // the newer name arrives first
    for op in &ops {
        receive(&replica, std::slice::from_ref(op)).await;
    }
    let rs = &replica.storage;
    assert_eq!(
        id_via(rs, B, "fr", "/produits/b", Index),
        Some(chair.clone())
    );
    assert_eq!(resolve(rs, "fr", "/produits/a"), None);
    assert_eq!(resolve(rs, "fr", "/produits/chaise"), None);
    // No live claim survives for the superseded names.
    assert!(claimants(rs, B, "fr", &products, "a").is_empty());
    assert!(claimants(rs, B, "fr", &products, "chaise").is_empty());
    assert_eq!(claimants(rs, B, "fr", &products, "b"), vec![chair]);
    let _ = Storage::localized_names(rs.as_ref());
}

/// A-B-A out of order: the origin names the chair `a` then `chaise` again;
/// the replica receives the newer write first. The newest state equals the
/// state before the late write, so a catch-up diffed against the STORED rows
/// alone (blind to the late write's own rows in the same batch) restaged
/// nothing, and the late write's `a` stayed HEAD with `chaise` dead: a 404
/// through a `Ready` index.
#[tokio::test]
async fn out_of_order_aba_apply_keeps_the_current_name() {
    let (origin, replica, start) = pair().await;
    let (products, chair) = catalog(&origin.storage).await;
    let shipped = highest(&origin);
    receive(&replica, &ops_after(&origin, start)).await;
    build(&replica.storage, B).await;

    set_name(&origin.storage, &chair, "fr", "a").await;
    set_name(&origin.storage, &chair, "fr", "chaise").await;
    let mut ops = ops_after(&origin, shipped);
    ops.reverse(); // r2 (chaise) arrives before r1 (a)
    for op in &ops {
        receive(&replica, std::slice::from_ref(op)).await;
    }
    let rs = &replica.storage;
    assert!(availability(rs, B).is_ready());
    assert_eq!(
        id_via(rs, B, "fr", "/produits/chaise", Index),
        Some(chair.clone())
    );
    assert_eq!(resolve(rs, "fr", "/produits/a"), None);
    assert!(claimants(rs, B, "fr", &products, "a").is_empty());
    assert_eq!(claimants(rs, B, "fr", &products, "chaise"), vec![chair]);
}

/// Plan Phase 13c: uniqueness is a LOCAL write rule. A replica that enforces
/// it applies a peer's overlay that collides here anyway — the peer accepted
/// it under its own policy, and refusing an already-committed revision
/// mid-apply would leave the replica diverged with no rollback (the
/// `allowed_children` / UNIQUE trust model). The next build counts it.
#[tokio::test]
async fn replicated_colliding_overlay_is_never_refused() {
    let (origin, replica, start) = pair().await;
    let products = create(&origin.storage, "/products", &[]).await;
    let chaise = create(&origin.storage, "/products/chaise", &[]).await;
    let table = create(&origin.storage, "/products/table", &[]).await;
    let shipped = highest(&origin);
    receive(&replica, &ops_after(&origin, start)).await;
    let rs = &replica.storage;
    let mut config = repo_config();
    config.localized_names.enforce_unique = true;
    set_config(rs, config).await;
    build(rs, B).await;
    assert!(availability(rs, B).is_ready());
    // Enforced on the replica: a LOCAL write of the same name is refused.
    let local = try_set_name(rs, &table, "fr", "chaise").await;
    assert!(
        matches!(local, Err(raisin_error::Error::Conflict(_))),
        "{local:?}"
    );

    // The origin does not enforce; its write replicates and is applied.
    set_name(&origin.storage, &table, "fr", "chaise").await;
    receive(&replica, &ops_after(&origin, shipped)).await;
    assert_eq!(
        claimants(rs, B, "fr", &products, "chaise"),
        vec![table.clone()]
    );

    // The applied collision is not a later LOCAL write's doing: writes of
    // either node that leave its names alone still land (they were all
    // refused until an operator renamed one).
    update_props(rs, &chaise, &[("title", "Chaise")]).await;
    set_name(rs, &table, "de", "tisch").await;
}
