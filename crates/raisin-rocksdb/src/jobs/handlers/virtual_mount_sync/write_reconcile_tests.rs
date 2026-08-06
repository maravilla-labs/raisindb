//! The durable change feed: the `RevisionMeta` watermark walk.
//!
//! Everything here is about what the drain's divergence check CANNOT see. The
//! headline case is a delete: the node is gone from the workspace and from the
//! sync index, so a sweep over live nodes cannot tell "deleted" from "never
//! existed". The only surviving evidence is the commit record plus the MVCC
//! pre-image, and both have a shelf life.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment and helpers.

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{MountConfig, MountState, WriteConfig, MAX_PENDING_DELETES};
use sync::write::reconcile::reconcile_mount;

const LIMIT: usize = 500;

/// A mount that asks for writes, so it earns a durable change feed.
///
/// `mirror`, because most of this file is about DELETES and a mirror is the only
/// mode with a delete path — `write::drain` drains the pending-delete queue in
/// `Mirror` mode and nowhere else. A `state_only` mount's feed is exercised by
/// [`state_only_mount`] below, which asserts the opposite property.
fn writable_mount() -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.write_config = WriteConfig {
        mode: "mirror".to_string(),
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    mount
}

/// The flagship shipped shape: a `state_only` mount, which pushes field updates
/// and never deletes anything at the provider.
fn state_only_mount() -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.write_config = WriteConfig {
        mode: "state_only".to_string(),
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    mount
}

/// State whose watermark is already established at the bottom, so a walk sees
/// everything that has ever been committed. Tests that care about
/// initialization set it up themselves.
fn walked_from_the_start() -> MountState {
    MountState {
        writeback_revision: Some(raisin_hlc::HLC::new(0, 0).to_string()),
        ..Default::default()
    }
}

/// Materialize one mount-owned node the way a sync would, returning its id.
async fn sync_in(mat: &RocksDbMaterializer, external_id: &str) -> String {
    let mut index = mat.load_index(&scope()).await.unwrap();
    mat.apply_batch(
        &scope(),
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: format!("{external_id}.txt"),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some(external_id.to_string()),
                properties: serde_json::Map::new(),
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: external_id.to_string(),
                etag: Some("v1".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();
    index
        .virtual_nodes()
        .into_iter()
        .find(|n| n.external_id == external_id)
        .expect("the imported node must be in the index")
        .id
}

/// Delete a node as an ordinary user would — a real transaction, a real actor,
/// a real `Deleted` change in the commit record.
async fn user_delete(env: &Env, ids: &[&str]) {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch("main").unwrap();
    tx.set_actor("alice").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_message("user delete").unwrap();
    for id in ids {
        tx.delete_node(TARGET_WS, id).await.unwrap();
    }
    tx.commit().await.unwrap();
}

/// Create a node that is NOT this mount's, under the mount path.
async fn user_create(env: &Env, name: &str, props: Vec<(&str, &str)>) -> String {
    let tx = begin(env).await;
    let mut node = Node {
        id: format!("user-{name}"),
        node_type: "raisin:Node".to_string(),
        name: name.to_string(),
        path: format!("{MOUNT_PATH}/{name}"),
        workspace: Some(TARGET_WS.to_string()),
        ..Default::default()
    };
    for (k, v) in props {
        node.properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    tx.upsert_deep_node(TARGET_WS, &node, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    node.id
}

// ---------------------------------------------------------------------------
// deletes: the whole reason this layer exists
// ---------------------------------------------------------------------------

/// The headline case. A user deletes a synced node; the walk recovers the
/// provider-side id from the MVCC pre-image.
///
/// Nothing else in the engine can do this. The node is gone from the workspace
/// listing, so `SyncIndex` has no entry, so `write::plan::candidates` has
/// nothing to nominate — a delete is not a "node whose fields diverge", it is
/// the absence of a node.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_delete_is_detected_with_its_external_id() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "DOC-1").await;

    let mut state = walked_from_the_start();
    // Walk past the import first, so only the delete is new.
    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    assert!(state.writeback_pending_deletes.is_empty());

    user_delete(&env, &[&id]).await;

    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert_eq!(found.deletes_found, 1, "the delete must be visible");
    let pending = &state.writeback_pending_deletes[0];
    assert_eq!(pending.node_id, id);
    assert_eq!(
        pending.external_id, "DOC-1",
        "the provider-side id has to come from the pre-image; nothing else still has it"
    );
    assert!(!pending.bulk);
}

/// Provenance (§9.3). A node under the mount path that this mount does not own
/// must never be reported as a provider delete: pushing it would delete
/// someone else's remote object, or an unrelated one.
#[tokio::test(flavor = "multi_thread")]
async fn a_delete_of_a_node_this_mount_never_owned_is_not_recorded() {
    let env = setup().await;

    let foreign = user_create(
        &env,
        "notes.txt",
        vec![("__mount_id", "some-other-mount"), ("__external_id", "X1")],
    )
    .await;
    let unmarked = user_create(&env, "plain.txt", vec![]).await;
    // Ours by `__mount_id`, but with no provider id — nothing to delete THERE.
    let idless = user_create(&env, "half.txt", vec![("__mount_id", MOUNT_ID)]).await;

    let mut state = walked_from_the_start();
    user_delete(&env, &[&foreign, &unmarked, &idless]).await;

    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert_eq!(found.deletes_found, 0);
    assert!(
        state.writeback_pending_deletes.is_empty(),
        "recorded a delete without provenance: {:?}",
        state.writeback_pending_deletes
    );
}

/// The bulk flag, which is free: `RevisionMeta.changed_nodes.len()` says how
/// wide the deleting transaction was. It is the only signal that separates a
/// person deleting a message from a mis-scoped `DELETE FROM …`, and it has to
/// be captured at DETECTION time — by the time anything drains, the commit that
/// carried it is behind the watermark.
#[tokio::test(flavor = "multi_thread")]
async fn a_mass_delete_is_flagged_bulk() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    let mut ids = Vec::new();
    for i in 0..(sync::config::BULK_REVISION_THRESHOLD + 5) {
        ids.push(sync_in(&mat, &format!("B{i}")).await);
    }
    let mut state = walked_from_the_start();
    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    let refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
    user_delete(&env, &refs).await;

    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert_eq!(state.writeback_pending_deletes.len(), ids.len());
    assert!(
        state.writeback_pending_deletes.iter().all(|p| p.bulk),
        "every delete from one wide transaction is part of a bulk delete"
    );
}

// ---------------------------------------------------------------------------
// what the walk must NOT see
// ---------------------------------------------------------------------------

/// Echo suppression, at commit granularity.
///
/// The sync's own writes must not read back as local edits, or the mount pushes
/// everything it just imported. The signal is `RevisionMeta.actor`, which the
/// materializer sets to `SYNC_ACTOR` — deliberately NOT the node's `updated_by`,
/// which reads `virtual-mount-sync` for those same writes and therefore cannot
/// distinguish them from anything else the engine touched.
#[tokio::test(flavor = "multi_thread")]
async fn the_syncs_own_writes_are_not_local_changes() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut state = walked_from_the_start();

    let id = sync_in(&mat, "ECHO").await;

    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert!(
        found.scanned > 0,
        "the import must have committed something"
    );
    assert!(
        found.echoes > 0,
        "the import commit must be recognized as an echo"
    );
    assert_eq!(
        found.local_changes, 0,
        "the sync's own import was counted as a local edit"
    );

    // And the same node edited by a person IS a local change.
    set_bool_prop(&env, &id, "unread", true).await;
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    assert_eq!(found.local_changes, 1);
}

/// A mount materializes into ONE branch. A commit on any other — a fork, a
/// publish target, the config branch — is not its business, and pushing it
/// would export a branch's private state to the provider.
#[tokio::test(flavor = "multi_thread")]
async fn a_commit_on_another_branch_is_ignored() {
    let env = setup().await;
    let mut mount = writable_mount();
    mount.target_branch = "release".to_string();

    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "MAIN-1").await; // materialized on `main`
    let mut state = walked_from_the_start();
    reconcile_mount(&env.storage, TENANT, REPO, &mount, &mut state, LIMIT)
        .await
        .unwrap();

    user_delete(&env, &[&id]).await; // deleted on `main`

    let found = reconcile_mount(&env.storage, TENANT, REPO, &mount, &mut state, LIMIT)
        .await
        .unwrap();

    assert!(found.scanned > 0, "the delete commit must have been read");
    assert_eq!(
        found.deletes_found, 0,
        "a delete on another branch is not ours"
    );
    assert_eq!(found.local_changes, 0);
}

// ---------------------------------------------------------------------------
// the watermark itself
// ---------------------------------------------------------------------------

/// A mount reconciling for the first time establishes its watermark at the
/// repository head instead of replaying history.
///
/// Replaying would surface every delete made before writeback existed — nodes
/// this mount could not have pushed, and in a repo with any history, an
/// unbounded amount of work on the very tick the operator enabled the feature.
#[tokio::test(flavor = "multi_thread")]
async fn the_first_pass_establishes_a_watermark_without_replaying_history() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "OLD").await;
    user_delete(&env, &[&id]).await;

    let mut state = MountState::default();
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert!(found.initialized);
    assert_eq!(
        found.scanned, 0,
        "history must not be walked on the first pass"
    );
    assert!(state.writeback_pending_deletes.is_empty());
    assert!(
        state.writeback_revision.is_some(),
        "the watermark must be established, or the next pass replays from zero"
    );

    // And from here on it works normally.
    let id2 = sync_in(&mat, "NEW").await;
    user_delete(&env, &[&id2]).await;
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    assert_eq!(found.deletes_found, 1);
}

/// The watermark is what makes the walk resumable AND idempotent: a second pass
/// over an unchanged repository reads nothing and finds nothing.
///
/// Without the advance, every pass re-walks from the bottom — the cost grows
/// with the repository forever, and every delete already recorded is recorded
/// again.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_pass_over_unchanged_history_does_no_work() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "DOC-2").await;
    let mut state = walked_from_the_start();
    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    user_delete(&env, &[&id]).await;
    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    assert_eq!(state.writeback_pending_deletes.len(), 1);

    let again = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert_eq!(again.scanned, 0, "the watermark did not advance");
    assert_eq!(again.deletes_found, 0);
    assert_eq!(
        state.writeback_pending_deletes.len(),
        1,
        "the same delete was recorded twice"
    );
}

/// The pending-delete cap is back-pressure, not truncation: the walk stops and
/// leaves the watermark where it is, so the deletes it did not record are found
/// again later. Dropping them would lose them permanently — this walk is the
/// only thing that can see them, and it gets one chance per revision.
#[tokio::test(flavor = "multi_thread")]
async fn the_pending_delete_cap_stops_the_walk_without_advancing_the_watermark() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "OVERFLOW").await;

    let mut state = walked_from_the_start();
    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    let watermark_before = state.writeback_revision.clone();

    // A backlog already at the cap.
    state.writeback_pending_deletes = (0..MAX_PENDING_DELETES)
        .map(|i| sync::config::PendingDelete {
            node_id: format!("old-{i}"),
            external_id: format!("OLD-{i}"),
            path: None,
            etag: None,
            revision: raisin_hlc::HLC::new(1, 0).to_string(),
            detected_at: 0,
            bulk: false,
        })
        .collect();

    user_delete(&env, &[&id]).await;
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert!(found.truncated);
    assert_eq!(found.deletes_found, 0);
    assert_eq!(
        state.writeback_revision, watermark_before,
        "the watermark advanced past a delete that was never recorded — it is now unrecoverable"
    );
    assert_eq!(state.writeback_pending_deletes.len(), MAX_PENDING_DELETES);

    // Drain the backlog, and the deferred delete is found on the next pass.
    state.writeback_pending_deletes.clear();
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    assert_eq!(found.deletes_found, 1);
    assert_eq!(state.writeback_pending_deletes[0].external_id, "OVERFLOW");
}

/// A mount with no delete path never queues a delete — and can therefore never
/// be wedged by the cap that governs the queue.
///
/// `write::drain` drains `writeback_pending_deletes` in `Mirror` mode and
/// nowhere else, so for a `state_only` (or `submit`) mount the queue has no
/// consumer at all. Recording into it made the queue append-only, and an
/// append-only queue behind back-pressure freezes the watermark PERMANENTLY once
/// it passes the cap: the walk breaks before advancing, on every tick, waiting
/// for a backlog nothing can drain. Durable change detection for the mount dies
/// there — no local edits, no boundary probes — and the only trace is a `warn!`
/// promising the opposite.
///
/// The second half is the same property for a mount that used to be a mirror:
/// the leftover queue is discarded rather than carried, so the wedge cannot be
/// inherited (and so flipping a mount to `mirror` later cannot retro-delete
/// remote objects for nodes deleted while it promised it would not).
#[tokio::test(flavor = "multi_thread")]
async fn a_state_only_mount_queues_no_deletes_and_is_never_wedged_by_the_cap() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "SO-1").await;

    let mut state = walked_from_the_start();
    reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &state_only_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();
    let watermark_before = state.writeback_revision.clone();

    user_delete(&env, &[&id]).await;
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &state_only_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert_eq!(
        found.deletes_found, 0,
        "a state_only mount has no delete path; queuing one only fills the queue"
    );
    assert!(state.writeback_pending_deletes.is_empty());
    assert_eq!(
        found.local_changes, 1,
        "the delete is still SEEN — detection is unchanged, only the queue is not fed"
    );
    assert!(!found.truncated);
    assert_ne!(
        state.writeback_revision, watermark_before,
        "the watermark must advance past the delete"
    );

    // A backlog inherited from an earlier `mirror` configuration is discarded
    // rather than carried — otherwise the mount stays wedged at the cap forever.
    state.writeback_pending_deletes = (0..MAX_PENDING_DELETES)
        .map(|i| sync::config::PendingDelete {
            node_id: format!("old-{i}"),
            external_id: format!("OLD-{i}"),
            path: None,
            etag: None,
            revision: raisin_hlc::HLC::new(1, 0).to_string(),
            detected_at: 0,
            bulk: false,
        })
        .collect();
    let watermark_before = state.writeback_revision.clone();
    let second = sync_in(&mat, "SO-2").await;
    user_delete(&env, &[&second]).await;

    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &state_only_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert!(
        state.writeback_pending_deletes.is_empty(),
        "a queue nothing can drain must not be carried"
    );
    assert!(
        !found.truncated,
        "the cap must not apply to a mount whose queue has no consumer"
    );
    assert_ne!(
        state.writeback_revision, watermark_before,
        "the watermark froze: durable change detection for this mount is dead"
    );
}

/// A watermark that cannot be parsed re-establishes at the head rather than
/// reading as zero. Reading it as zero would replay the whole repository, and
/// every delete in it, on a mount that had been writing happily.
#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_watermark_re_establishes_rather_than_replaying() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in(&mat, "DOC-3").await;
    user_delete(&env, &[&id]).await;

    let mut state = MountState {
        writeback_revision: Some("not-an-hlc".to_string()),
        ..Default::default()
    };
    let found = reconcile_mount(
        &env.storage,
        TENANT,
        REPO,
        &writable_mount(),
        &mut state,
        LIMIT,
    )
    .await
    .unwrap();

    assert!(found.initialized);
    assert_eq!(found.scanned, 0);
    assert!(state.writeback_pending_deletes.is_empty());
    assert!(state
        .writeback_revision
        .as_deref()
        .unwrap()
        .parse::<raisin_hlc::HLC>()
        .is_ok());
}

// ---------------------------------------------------------------------------
// which mounts get a change feed at all
// ---------------------------------------------------------------------------

/// Detection follows the mount's INTENT to write, not the engine's current
/// ability to honour it. A `submit` mount cannot push, and neither can a mirror
/// whose adapter falls short — but their deletes still have to be recorded while
/// the pre-image exists, or the first mount to gain a working mode has no
/// history to act on.
#[test]
fn any_declared_write_mode_earns_a_change_feed() {
    let feed = |mode: &str, writeback: &str| {
        WriteConfig {
            mode: mode.to_string(),
            writeback: writeback.to_string(),
            ..Default::default()
        }
        .wants_any_write()
    };

    assert!(feed("state_only", "off"));
    assert!(feed("mirror", "off"));
    assert!(feed("submit", "off"));
    assert!(
        feed("off", "write_through"),
        "the deprecated alias still asks for writes"
    );

    assert!(!feed("off", "off"));
    assert!(!feed("", "off"));
    // A typo says nothing about wanting writes, and `unsupported_mode_reason`
    // already refuses it out loud.
    assert!(!feed("mirrror", "off"));
}
