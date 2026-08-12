//! `conflict: local_wins` on the READ path: an incoming remote item must not
//! revert a pending local edit.
//!
//! The write path has honoured `local_wins` since the conflict module existed —
//! but only when the provider answered a push with 409. The read path used to
//! rebuild the node wholesale from mapper output and reseed `__pushed_state`
//! from the incoming item, so any delta arriving between a local edit and a
//! successful drain both reverted the edit AND de-nominated it: no error, no
//! failed count, the edit simply never reached the provider. For a provider
//! whose etag moves on the very field being edited (Graph's `isRead` moves
//! `@odata.etag`) that window is effectively always open, which is the reported
//! "is_read is NEVER written back" symptom.
//!
//! Every test here asserts the OBSERVABLE consequence — the value on the node,
//! the baseline entry, the update that does or does not reach the mock — never
//! an internal flag. A child module of [`super::tests`] (declared with
//! `#[path]` at the bottom of `tests.rs`) so it can reuse that file's
//! environment, mocks and helpers.

use serde_json::{json, Value};

use super::*;
/// The engine module under test. `super` is `tests` here, so engine items are
/// one level further out than they are in `tests.rs` itself.
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{MappedNode, MountState};
use sync::materializer::{BatchOp, VirtualMeta, PUSHED_STATE_PROP};

/// The `state_only` mode these drains run under.
fn state_only_mode() -> sync::write::WriteMode {
    sync::write::WriteMode::StateOnly(sync::write::FieldPlan::pushing(&["unread"]))
}

/// `watched_scope()` with the read-path `local_wins` flag on — what
/// `build_ctx_parts` derives for a mount whose `write_config.conflict` is an
/// explicit, well-formed `local_wins`.
fn local_wins_scope() -> MountScope {
    MountScope {
        read_local_wins: true,
        ..watched_scope()
    }
}

/// `__pushed_state` as stored, or `None`.
fn maybe_pushed_state(node: &Node) -> Option<Value> {
    node.properties
        .get(PUSHED_STATE_PROP)
        .map(|pv| serde_json::to_value(pv).unwrap())
}

fn json_prop(node: &Node, key: &str) -> Option<Value> {
    node.properties
        .get(key)
        .map(|pv| serde_json::to_value(pv).unwrap())
}

/// Deliver one mail-shaped item under `scope` the way a delta would, returning
/// the node id. Arbitrary properties, so the two-field tests can exercise a
/// diverged and a non-diverged field in the same item.
async fn deliver(
    mat: &RocksDbMaterializer,
    scope: &MountScope,
    external_id: &str,
    properties: serde_json::Map<String, Value>,
    etag: &str,
) -> String {
    let mut index = mat.load_index(scope).await.unwrap();
    mat.apply_batch(
        scope,
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: format!("{external_id}.eml"),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some(external_id.to_string()),
                properties,
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: external_id.to_string(),
                etag: Some(etag.to_string()),
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
        .expect("the delivered node must be in the index")
        .id
}

fn unread(value: bool) -> serde_json::Map<String, Value> {
    let mut properties = serde_json::Map::new();
    properties.insert("unread".to_string(), Value::Bool(value));
    properties
}

// ---------------------------------------------------------------------------
// (a) the edit survives the incoming item, and the next drain pushes it
// ---------------------------------------------------------------------------

/// Under `local_wins` a pending local edit survives a remote-item upsert — and
/// the divergence survives with it, so the next drain still nominates and
/// pushes the field.
///
/// This is the ms-graph `is_read` scenario end to end: local toggle, then a
/// delta whose etag moved (Graph moves `@odata.etag` on the toggle itself),
/// then the drain. Note what the drain sends as its concurrency base: the etag
/// of the item we knowingly overrode — the read path kept it fresh, so the
/// push succeeds cleanly without ever reaching the 409 force path.
#[tokio::test(flavor = "multi_thread")]
async fn a_pending_edit_survives_an_incoming_item_and_is_pushed() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    // Synced in read, baseline seeded from the provider's own report.
    let id = deliver(&mat, &local_wins_scope(), "M1", unread(false), "v1").await;

    // The user marks it unread...
    set_bool_prop(&env, &id, "unread", true).await;

    // ...and a delta arrives first: same item, new etag, remote still `false`.
    let same = deliver(&mat, &local_wins_scope(), "M1", unread(false), "v2").await;
    assert_eq!(same, id, "the delta must update the same node");

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        json_prop(&node, "unread"),
        Some(Value::Bool(true)),
        "the local edit must survive the rebuild"
    );
    assert_eq!(
        maybe_pushed_state(&node),
        Some(json!({ "unread": false })),
        "the baseline must keep the OLD entry, or the divergence is lost"
    );
    assert_eq!(
        str_prop(&node, "__etag").as_deref(),
        Some("v2"),
        "the stored etag names the remote version we knowingly overrode"
    );

    // The next drain pushes the preserved edit — once, against the fresh etag.
    let mut mount = state_only_mount();
    mount.write_config.conflict = Some("local_wins".to_string());
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(
        stats.pushed, 1,
        "the preserved edit must reach the provider"
    );
    assert_eq!(stats.failed, 0);
    assert_eq!(mock.update_count(), 1);
    let sent = mock.updates.lock().unwrap()[0].clone();
    assert_eq!(sent.get("payload"), Some(&json!({ "isRead": false })));
    assert_eq!(
        sent.get("etag").and_then(|v| v.as_str()),
        Some("v2"),
        "the concurrency base is the incoming item's etag, not the stale one"
    );

    // And it converges: baseline reseeded from the confirmed push.
    let node = node_by_id(&env, &id).await;
    assert_eq!(maybe_pushed_state(&node), Some(json!({ "unread": true })));
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 1, "the second drain is a no-op");
}

// ---------------------------------------------------------------------------
// (b) remote_wins / policy-unset behaviour is unchanged
// ---------------------------------------------------------------------------

/// Without `local_wins` the incoming item still wins, exactly as before: the
/// node follows the remote value, the baseline reseeds from the item, and the
/// next drain has nothing to send.
///
/// The mirror of `write_baseline_tests::
/// a_watching_mount_still_reseeds_the_baseline_from_the_remote_item`, restated
/// here with the local edit in the window so the REVERT itself is the thing
/// asserted — that is `remote_wins`' documented trade (the recoverable half:
/// the remote object is untouched and the user can redo the edit), and this
/// change must not have moved it.
#[tokio::test(flavor = "multi_thread")]
async fn remote_wins_still_reverts_the_pending_edit_on_arrival() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    // `watched_scope()` carries `read_local_wins: false` — the default every
    // policy but an explicit `local_wins` resolves to.
    let id = deliver(&mat, &watched_scope(), "M1", unread(false), "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;
    deliver(&mat, &watched_scope(), "M1", unread(false), "v2").await;

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        json_prop(&node, "unread"),
        Some(Value::Bool(false)),
        "under remote_wins the incoming item overwrites the local edit"
    );
    assert_eq!(
        maybe_pushed_state(&node),
        Some(json!({ "unread": false })),
        "and the baseline reseeds from the item"
    );

    // Reverted AND de-nominated: nothing diverges, nothing is pushed.
    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0);
}

// ---------------------------------------------------------------------------
// (c) non-diverged fields still converge under local_wins
// ---------------------------------------------------------------------------

/// `local_wins` preserves EXACTLY the pending fields. A second watched field
/// with no local edit follows the remote item and its baseline reseeds — the
/// preserve step must not become a preserve-always, or every inbound change on
/// a `local_wins` mount would look like a local edit and be pushed straight
/// back at the provider that reported it.
#[tokio::test(flavor = "multi_thread")]
async fn non_diverged_watched_fields_still_reseed_from_the_item() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let scope = MountScope {
        watched_fields: vec!["unread".to_string(), "folder".to_string()],
        read_local_wins: true,
        ..scope()
    };

    let mut props = unread(false);
    props.insert("folder".to_string(), Value::String("inbox".to_string()));
    let id = deliver(&mat, &scope, "M1", props, "v1").await;

    // Local edit touches `unread` only.
    set_bool_prop(&env, &id, "unread", true).await;

    // The remote moves the mail to another folder in the same window.
    let mut props = unread(false);
    props.insert("folder".to_string(), Value::String("archive".to_string()));
    deliver(&mat, &scope, "M1", props, "v2").await;

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        json_prop(&node, "unread"),
        Some(Value::Bool(true)),
        "the edited field keeps the local value"
    );
    assert_eq!(
        json_prop(&node, "folder"),
        Some(Value::String("archive".to_string())),
        "the untouched field follows the remote"
    );
    assert_eq!(
        maybe_pushed_state(&node),
        Some(json!({ "unread": false, "folder": "archive" })),
        "the baseline keeps the old entry for the pending field only; the \
         non-diverged one reseeds so it is not pushed back next drain"
    );
}

/// When the incoming item already carries the locally-edited value the field is
/// no longer pending: the baseline reseeds from the item and the next drain has
/// nothing to send. The remote holding the local value IS convergence, and
/// preserving the stale baseline instead would queue a pointless no-op push.
#[tokio::test(flavor = "multi_thread")]
async fn an_item_already_matching_the_local_edit_converges() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    let id = deliver(&mat, &local_wins_scope(), "M1", unread(false), "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;
    // The other client's write beat ours to the provider, or our own drain's
    // stamp raced: either way the remote now reports the edited value.
    deliver(&mat, &local_wins_scope(), "M1", unread(true), "v2").await;

    let node = node_by_id(&env, &id).await;
    assert_eq!(json_prop(&node, "unread"), Some(Value::Bool(true)));
    assert_eq!(
        maybe_pushed_state(&node),
        Some(json!({ "unread": true })),
        "the remote already holds the local value; reseeding is the convergence"
    );

    let mut mount = state_only_mount();
    mount.write_config.conflict = Some("local_wins".to_string());
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0, "nothing left to push");
}

// ---------------------------------------------------------------------------
// the remap inherits the preserve step
// ---------------------------------------------------------------------------

/// A `force_rewrite` run (remap) on a `local_wins` mount keeps pending edits
/// too. The remap bypasses the etag skip and re-materializes every item, so
/// without inheriting the preserve step it would clobber pending edits exactly
/// like a delta — and its contract is "re-apply the mapper to unchanged remote
/// data", not "resolve conflicts in remote's favour".
#[tokio::test(flavor = "multi_thread")]
async fn a_remap_keeps_the_pending_edit_too() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    let id = deliver(&mat, &local_wins_scope(), "M1", unread(false), "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    // Remap: same item, SAME etag — which an ordinary sync would skip and a
    // remap deliberately re-applies.
    let remap = MountScope {
        force_rewrite: true,
        ..local_wins_scope()
    };
    deliver(&mat, &remap, "M1", unread(false), "v1").await;

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        json_prop(&node, "unread"),
        Some(Value::Bool(true)),
        "a remap re-projects the mapper, it does not resolve conflicts"
    );
    assert_eq!(maybe_pushed_state(&node), Some(json!({ "unread": false })));
}
