//! An update that RENAMES a key-addressed object reports a new external id.
//!
//! On S3 the key IS the identity: a rename is a copy to a new key, so an update
//! that renames leaves the engine holding an id that resolves to nothing. The
//! drain used to re-stamp `candidate.external_id` unconditionally and read only
//! `etag` from the adapter's answer, so the adapter had no way to say so — the
//! next reconcile found nothing under the old id, pruned the node, and
//! re-imported the same object as a fresh one with a new node id, no history and
//! no pending local edits.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use serde_json::{json, Value};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::MountState;

fn state_only_mode() -> sync::write::WriteMode {
    sync::write::WriteMode::StateOnly(sync::write::FieldPlan::pushing(&["unread"]))
}

/// Sync one node in, edit it locally, and drain — with the adapter answering
/// `update` with `reply`. Returns the node id and the run's stats.
async fn drain_with_update_reply(
    env: &Env,
    mat: &RocksDbMaterializer,
    reply: Value,
) -> (String, sync::write::DrainStats) {
    let mock = StateOnlyMock::new();
    mock.set_update_reply(reply);
    let id = sync_in_mail(mat, false, "v1").await;
    set_bool_prop(env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(env, &mount, &mock, mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    batcher.flush().await.unwrap();
    (id, stats)
}

/// The headline: the node keeps its identity in RaisinDB and changes it at the
/// provider.
#[tokio::test(flavor = "multi_thread")]
async fn an_update_reporting_a_new_external_id_re_keys_the_node() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let (id, stats) =
        drain_with_update_reply(&env, &mat, json!({ "external_id": "M2", "etag": "v2" })).await;

    assert_eq!(stats.pushed, 1, "a rename is still a completed push");
    assert_eq!(stats.failed, 0);

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        str_prop(&node, "__external_id").as_deref(),
        Some("M2"),
        "the node must now name the object the provider actually has"
    );
    assert_eq!(
        node.id, id,
        "and it must be the SAME node — re-keying exists precisely so the node \
         id, its history and its local edits survive a rename"
    );
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v2"));
    assert_eq!(
        pushed_state(&node),
        json!({ "unread": true }),
        "the push landed, so the baseline moves with it"
    );
}

/// The index entry moves too, so nothing later in the SAME run can still reach
/// the node under an id the provider no longer has.
///
/// The concrete hazard is the one a key-addressed rename actually produces: a
/// rename is a copy plus a delete, so the very next change page can carry
/// `deleted OLD`. With a stale index entry that tombstone destroys the node that
/// was just re-keyed — the rename would delete the file.
#[tokio::test(flavor = "multi_thread")]
async fn the_old_index_entry_is_dropped_so_a_stale_tombstone_cannot_delete_the_node() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    mock.set_update_reply(json!({ "external_id": "M2", "etag": "v2" }));

    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    batcher.flush().await.unwrap();

    // The provider reports the OLD key gone, in the same run, through the same
    // index the drain just mutated.
    batcher.stage_delete("M1").await.unwrap();
    batcher.flush().await.unwrap();

    let survivors = virtual_assets(&all_nodes(&env, TARGET_WS).await)
        .iter()
        .map(|n| n.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        survivors,
        vec![id],
        "the re-keyed node must not be reachable under the id it no longer has"
    );
}

/// An id that did not change is not a re-key, and must go through the ordinary
/// stamp — every adapter that echoes `item_id` back (the mirror mock does) would
/// otherwise take the loud path on every single push.
#[tokio::test(flavor = "multi_thread")]
async fn echoing_the_same_external_id_back_is_not_a_re_key() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let (id, stats) =
        drain_with_update_reply(&env, &mat, json!({ "external_id": "M1", "etag": "v2" })).await;

    assert_eq!(stats.pushed, 1);
    let node = node_by_id(&env, &id).await;
    assert_eq!(str_prop(&node, "__external_id").as_deref(), Some("M1"));
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v2"));
}

/// An EMPTY external id is an adapter bug, and acting on it would orphan the
/// node: a node whose `__external_id` is blank is invisible to the index and, to
/// the delete rails, not mount-owned at all. The push still counts — the
/// provider did the work — but the identity is left alone.
#[tokio::test(flavor = "multi_thread")]
async fn an_empty_external_id_is_refused_rather_than_obeyed() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let (id, stats) =
        drain_with_update_reply(&env, &mat, json!({ "external_id": "", "etag": "v2" })).await;

    assert_eq!(stats.pushed, 1);
    let node = node_by_id(&env, &id).await;
    assert_eq!(
        str_prop(&node, "__external_id").as_deref(),
        Some("M1"),
        "an empty id must never replace a working one"
    );
}

/// An answer with no `external_id` at all — every adapter shipped today — is
/// unchanged behaviour.
#[tokio::test(flavor = "multi_thread")]
async fn an_answer_without_an_external_id_stamps_exactly_as_before() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let (id, stats) = drain_with_update_reply(&env, &mat, json!({ "etag": "v2" })).await;

    assert_eq!(stats.pushed, 1);
    let node = node_by_id(&env, &id).await;
    assert_eq!(str_prop(&node, "__external_id").as_deref(), Some("M1"));
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v2"));
    assert_eq!(pushed_state(&node), json!({ "unread": true }));
}

/// Materialize a SECOND mount-owned node, already holding the id the provider is
/// about to report — the destination of a rename onto an occupied key.
///
/// Seeded through `apply_batch` like [`sync_in_mail`], so it carries a converged
/// `__pushed_state` and is therefore not itself a push candidate: the drain must
/// push exactly the one node the test edits.
async fn sync_in_second_mail(mat: &RocksDbMaterializer, external_id: &str, rel_path: &str) {
    let mut index = mat.load_index(&watched_scope()).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert("unread".to_string(), Value::Bool(false));
    mat.apply_batch(
        &watched_scope(),
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: rel_path.to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some(rel_path.to_string()),
                properties,
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
}

/// A rename onto a key another node already mirrors is REFUSED.
///
/// On a key-addressed store, writing to an existing key is an overwrite, so this
/// is reachable. Obeying it would leave two nodes carrying one `__external_id` —
/// and `SyncIndex::from_nodes` keys `by_external` by that id, so the next run
/// would see only one of them. The other could never again be matched by a
/// delta, reported as seen, or reconciled away: a duplicate no run can clear.
#[tokio::test(flavor = "multi_thread")]
async fn a_re_key_onto_an_id_another_node_already_holds_is_refused() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    mock.set_update_reply(json!({ "external_id": "M2", "etag": "v2" }));

    let id = sync_in_mail(&mat, false, "v1").await;
    sync_in_second_mail(&mat, "M2", "m2.eml").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    batcher.flush().await.unwrap();

    assert_eq!(
        stats.pushed, 1,
        "the provider did the work; the push counts"
    );
    let node = node_by_id(&env, &id).await;
    assert_eq!(
        str_prop(&node, "__external_id").as_deref(),
        Some("M1"),
        "the edited node must keep its own id rather than collide with m2's"
    );
    // Both nodes are still distinguishable, which is the whole point.
    let mut ids: Vec<String> = virtual_assets(&all_nodes(&env, TARGET_WS).await)
        .iter()
        .filter_map(|n| str_prop(n, "__external_id"))
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["M1".to_string(), "M2".to_string()]);
}
