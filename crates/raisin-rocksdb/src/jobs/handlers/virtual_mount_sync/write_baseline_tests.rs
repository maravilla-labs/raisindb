//! Integrity of the writeback baseline (`__pushed_state`): what may claim a
//! value was pushed, what must survive a config change, and what a stamp counts
//! as.
//!
//! All three failures this covers are silent by construction — no error, no
//! failed count, `writeback_supported: true` throughout — which is why each has
//! a test asserting the OBSERVABLE consequence (an edit that reaches the
//! provider, a property that is still there, a run that gives up) rather than
//! the internal flag.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

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

/// `__pushed_state` as stored, or `None` — unlike `pushed_state`, which asserts
/// it is there. Absence is the thing under test here.
fn maybe_pushed_state(node: &Node) -> Option<Value> {
    node.properties
        .get(PUSHED_STATE_PROP)
        .map(|pv| serde_json::to_value(pv).unwrap())
}

/// Import one mail-shaped node under an arbitrary scope, returning its node id.
///
/// Takes the scope so a test can import a node the way a mount that watches
/// NOTHING would — which is the only way to produce a node with no
/// `__pushed_state`, and exactly the state every node imported before writeback
/// was configured is in.
async fn import_mail(
    mat: &RocksDbMaterializer,
    scope: &MountScope,
    external_id: &str,
    unread: bool,
    etag: &str,
) -> String {
    let mut index = mat.load_index(scope).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert("unread".to_string(), Value::Bool(unread));
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
        .expect("the imported node must be in the index")
        .id
}

// ---------------------------------------------------------------------------
// 1. the seed path (defect 2)
// ---------------------------------------------------------------------------

/// An edit made while writeback was OFF is pushed once writeback is on.
///
/// The drain used to "seed" a node with no `__pushed_state` by stamping its
/// CURRENT LOCAL values as the baseline, with no adapter call. That is an
/// assertion about the provider the engine never verified — the remote values
/// are not in scope anywhere in the drain and there is no adapter `get` — and
/// when it is wrong the user's edit is recorded as already-pushed, `diverges()`
/// answers false forever, and nothing ever sends it. Silent loss, on a mount
/// reporting `writeback_supported: true` with zero failures.
///
/// The edit below is made BEFORE the mode is switched to `state_only`, which is
/// the ordinary rollout order: a mailbox has been synced read-only for months,
/// people have been marking things read in RaisinDB the whole time, and then
/// writeback is enabled.
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_made_before_writeback_was_enabled_is_still_pushed() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    // Imported while the mount watched nothing: no baseline at all.
    let id = import_mail(&mat, &scope(), "OLD", false, "v1").await;
    assert_eq!(maybe_pushed_state(&node_by_id(&env, &id).await), None);

    // The user marks it unread, with writeback still off.
    set_bool_prop(&env, &id, "unread", true).await;

    // Writeback is switched on.
    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.pushed, 1, "the pending edit must reach the provider");
    assert_eq!(stats.failed, 0);
    assert_eq!(mock.update_count(), 1);
    let sent = mock.updates.lock().unwrap()[0].clone();
    assert_eq!(
        sent.get("payload"),
        Some(&json!({ "isRead": false })),
        "the value sent must be the user's edit (unread), not the imported one"
    );
    assert_eq!(
        sent.get("etag").and_then(|v| v.as_str()),
        Some("v1"),
        "the stored etag is the concurrency base, so a remote that moved since \
         the last sync is refused rather than overwritten"
    );

    // Only NOW is a baseline recorded — of a value the provider confirmed.
    let node = node_by_id(&env, &id).await;
    assert_eq!(maybe_pushed_state(&node), Some(json!({ "unread": true })));
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v2"));

    // And it converges: the burst is one write per pre-existing node, once.
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 1, "the second drain is a no-op");
}

/// A node with no baseline and no watched value present is left alone.
///
/// The counterpart to the test above: "unseeded" must not mean "push
/// something". With no local value for the watched field there is nothing to
/// send, and the node costs no provider call and no revision — on this run or
/// any later one.
#[tokio::test(flavor = "multi_thread")]
async fn an_unseeded_node_with_nothing_to_say_is_not_pushed() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    // Imported with no `unread` property at all.
    let mut index = mat.load_index(&scope()).await.unwrap();
    mat.apply_batch(
        &scope(),
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: "quiet.eml".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some("quiet".to_string()),
                properties: serde_json::Map::new(),
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: "QUIET".to_string(),
                etag: Some("v1".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0);
}

// ---------------------------------------------------------------------------
// 2. the baseline across a config change (defect 3)
// ---------------------------------------------------------------------------

/// Turning writeback off does not destroy what was already pushed.
///
/// `watched_fields` is empty for every mode but `state_only`, and an upsert
/// rebuilds a node's property map from mapper output — so a single delta run
/// with `mode: "off"` used to strip `__pushed_state` from every node it
/// touched. Nothing said so. Re-enabling writeback then met a mailbox of nodes
/// with no record of what had been pushed, which is precisely the state defect 2
/// is about: every one of them diverges and is re-sent.
#[tokio::test(flavor = "multi_thread")]
async fn a_recorded_baseline_survives_the_mode_being_turned_off() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    // A `state_only` run records the baseline.
    let id = import_mail(&mat, &watched_scope(), "M1", false, "v1").await;
    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": false }))
    );

    // The operator sets `mode: "off"` to debug something. The mount now watches
    // nothing, and the provider reports the same item with a new etag.
    let same_id = import_mail(&mat, &scope(), "M1", false, "v2").await;
    assert_eq!(same_id, id, "the delta must update the same node");

    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": false })),
        "a mode change must not delete the record of what was pushed"
    );

    // Which is what keeps re-enabling writeback quiet: the node is converged,
    // not unseeded, so it is not re-sent to the provider.
    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0);
}

/// A watching mount still RE-SEEDS the baseline from the item the provider just
/// reported.
///
/// The carry-forward must not become a preserve-always: a remote change has to
/// converge on arrival, or every inbound edit looks like a local one and is
/// pushed straight back at the provider that reported it. The distinction is
/// "the engine has nothing to say" (no watched fields → carry) versus "the
/// engine says these are the values" (watched fields → overwrite, even with an
/// empty map).
#[tokio::test(flavor = "multi_thread")]
async fn a_watching_mount_still_reseeds_the_baseline_from_the_remote_item() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    let id = import_mail(&mat, &watched_scope(), "M1", false, "v1").await;
    // The remote flips it: same node, new etag, new value.
    import_mail(&mat, &watched_scope(), "M1", true, "v2").await;

    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": true })),
        "the item the provider just reported IS the new baseline"
    );
}

// ---------------------------------------------------------------------------
// 3. stamps and the failure budget (defect 4)
// ---------------------------------------------------------------------------

/// A drain's stamps do not count as items written, so a mount that cannot
/// materialize anything still gives up.
///
/// `check_failure_budget` gives up only while `written == 0` — "this mount has
/// written nothing, so these are not unlucky items". A stamp used to increment
/// that counter, so any mount with a drain in front of its read phases was
/// permanently exempt: it ground through the whole item budget, burned the 600s
/// job timeout, was retried three times and reported OK. That is the exact
/// incident the guard was built for, re-opened by the write path.
#[tokio::test(flavor = "multi_thread")]
async fn a_stamp_does_not_disarm_the_wholesale_rejection_guard() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    let node_id = import_mail(&mat, &watched_scope(), "M1", false, "v1").await;

    let mut mount = state_only_mount();
    mount.sync_config.max_item_failures = 3;
    let c = ctx(&env, &mount, &mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();

    // The drain's half: a stamp-back that lands.
    let mut pushed = serde_json::Map::new();
    pushed.insert("unread".to_string(), Value::Bool(false));
    batcher
        .stage_stamp(
            &node_id,
            "M1",
            Some("v2".to_string()),
            Some(pushed),
            None,
            512,
        )
        .await
        .unwrap();
    batcher.flush().await.unwrap();
    let after_stamp = batcher.stats();
    assert_eq!(after_stamp.stamped, 1);
    assert_eq!(
        after_stamp.written, 0,
        "a stamp amends the engine's own metadata; it is not an item imported"
    );

    // The read half: a mapper producing a node type this repo does not have, so
    // every item is rejected identically — the shape of a mount pointed at the
    // wrong workspace.
    for i in 0..3 {
        let item: sync::config::ExternalItem = serde_json::from_value(json!({
            "external_id": format!("BAD{i}"),
            "name": format!("bad{i}"),
            "is_folder": false,
            "etag": "v1",
        }))
        .unwrap();
        batcher
            .stage_upsert(
                &item,
                &format!("bad{i}.eml"),
                MappedNode {
                    node_type: "raisin:NotARealNodeType".to_string(),
                    name: Some(format!("bad{i}")),
                    properties: serde_json::Map::new(),
                },
            )
            .await
            .unwrap();
    }
    let err = batcher
        .flush()
        .await
        .expect_err("three wholesale rejections with nothing written must give up");
    assert!(
        matches!(err, sync::AdapterError::Config(_)),
        "and give up as a CONFIG error, which finalize reports as `misconfigured` \
         and the job layer does not retry: {err:?}"
    );
    let stats = batcher.stats();
    assert_eq!(stats.failed, 3);
    assert_eq!(stats.written, 0);
    assert_eq!(
        stats.stamped, 1,
        "the stamp still happened, and still counts"
    );
}
