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

/// A SMALL mount that rejects everything is caught too.
///
/// `check_failure_budget` is an early abort and waits for `max_item_failures`,
/// 500 by default. A mount with four items therefore never reaches it: the run
/// ended `outcome: "ok"` with `written: 0, failed: 4`, and the operator saw a
/// green mount that had imported nothing. That is not a hypothetical — it is
/// what a Stripe products mount did in production against a workspace whose
/// `allowed_node_types` did not list `stripe:Product`.
///
/// The damage outlived the run. The walk had reached the end without being
/// truncated or stopped, so `backfill_complete` was set and the mount switched
/// to delta-only; the change feed does not replay objects that already existed,
/// so those four items could never arrive again even after the workspace was
/// fixed. `check_completed_walk` closes both halves: once the listing is
/// exhausted, "wrote nothing while rejecting something" is conclusive at any
/// scale, and it returns before the flag is set.
#[tokio::test(flavor = "multi_thread")]
async fn a_four_item_mount_that_rejects_everything_is_not_reported_ok() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    // The DEFAULT budget, not a lowered one — the whole point is that four is
    // nowhere near it.
    let mount = state_only_mount();
    assert!(
        mount.sync_config.max_item_failures > 4,
        "this test is meaningless unless the early-abort threshold is out of reach"
    );

    let c = ctx(&env, &mount, &mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();

    for i in 0..4 {
        let item: sync::config::ExternalItem = serde_json::from_value(json!({
            "external_id": format!("prod_{i}"),
            "name": format!("prod_{i}"),
            "is_folder": false,
            "etag": "v1",
        }))
        .unwrap();
        batcher
            .stage_upsert(
                &item,
                &format!("prod_{i}"),
                MappedNode {
                    node_type: "stripe:Product".to_string(),
                    name: Some(format!("prod_{i}")),
                    properties: serde_json::Map::new(),
                },
            )
            .await
            .unwrap();
    }

    // The flush itself must NOT fail: four is under the early-abort threshold,
    // and that behaviour is deliberate — a big import must not die on its first
    // few bad items.
    batcher
        .flush()
        .await
        .expect("four rejections is under the early-abort budget");
    let stats = batcher.stats();
    assert_eq!(stats.written, 0);
    assert_eq!(stats.failed, 4);

    // The end-of-walk judgement is where it is caught.
    let err = batcher.check_completed_walk().expect_err(
        "a completed walk that wrote nothing while rejecting four items is misconfigured",
    );
    assert!(
        matches!(err, sync::AdapterError::Config(_)),
        "must be Config so `finalize` marks the mount `misconfigured` and does not retry: {err:?}"
    );
    assert!(
        format!("{err}").contains("all 4 item(s) were rejected"),
        "the message must name the scale it caught: {err}"
    );
}

/// An empty walk is not a failure.
///
/// A provider with nothing to offer writes nothing and rejects nothing, which
/// must stay a perfectly ordinary successful run — the checkout-sessions mount
/// starts exactly here.
#[tokio::test(flavor = "multi_thread")]
async fn a_walk_that_found_nothing_is_still_ok() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();

    batcher
        .check_completed_walk()
        .expect("an empty listing is not a misconfiguration");
}

/// A HEALTHY mount with one bad item is not condemned.
///
/// This is the regression the end-of-walk guard could easily have introduced.
/// `written` counts upserts that actually landed, so a mount in steady state
/// writes nothing on a re-walk — every item is unchanged and skipped on its
/// etag. Judging on "nothing written and something failed" alone would mark a
/// perfectly working mount `misconfigured` and stop it the moment one item went
/// bad: thousands skipped, one rejected, nothing written.
#[tokio::test(flavor = "multi_thread")]
async fn a_steady_state_mount_with_one_bad_item_is_left_alone() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();

    // One good item, imported and then re-offered unchanged so it skips.
    let good: sync::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "GOOD1", "name": "good1", "is_folder": false, "etag": "v1",
    }))
    .unwrap();
    let mapped = || MappedNode {
        node_type: "raisin:Node".to_string(),
        name: Some("good1".to_string()),
        properties: serde_json::Map::new(),
    };
    batcher
        .stage_upsert(&good, "good1", mapped())
        .await
        .unwrap();
    batcher.flush().await.unwrap();
    batcher
        .stage_upsert(&good, "good1", mapped())
        .await
        .unwrap();

    // ...and one item the workspace will not accept.
    let bad: sync::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "BAD1", "name": "bad1", "is_folder": false, "etag": "v1",
    }))
    .unwrap();
    batcher
        .stage_upsert(
            &bad,
            "bad1",
            MappedNode {
                node_type: "raisin:NotARealNodeType".to_string(),
                name: Some("bad1".to_string()),
                properties: serde_json::Map::new(),
            },
        )
        .await
        .unwrap();
    batcher.flush().await.unwrap();

    let stats = batcher.stats();
    assert!(
        stats.skipped > 0,
        "the good item must have skipped: {stats:?}"
    );
    assert!(stats.failed > 0, "the bad item must have failed: {stats:?}");

    batcher
        .check_completed_walk()
        .expect("a mount that is skipping real items is working, not misconfigured");
}

// ---------------------------------------------------------------------------
// the outbox lifecycle survives re-materialization
// ---------------------------------------------------------------------------

/// A re-sync must not erase the record that a command was sent.
///
/// A command node is BOTH a command and a synced item, and an upsert rebuilds
/// the property map from mapper output. The submit lifecycle is engine-written
/// and appears nowhere in that output, so the first sync after a send erased it:
/// a paid Stripe checkout session went from `status: sent` to no status at all,
/// taking `attempt_id`, `sent_at` and `sent_external_id` with it.
///
/// It never double-charged — the drain claims only `queued` — but it destroyed
/// exactly the states that matter most. `unknown` means "this may or may not
/// have charged someone", and erasing it makes an ambiguous command look fresh;
/// `attempt_id` is what makes such a case answerable at the provider at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_re_sync_does_not_erase_the_outbox_lifecycle() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let scope = MountScope {
        command_node_types: vec!["raisin:Node".to_string()],
        ..watched_scope()
    };

    // Materialized by the sync, so the index genuinely owns it.
    let id = import_mail(&mat, &scope, "CMD1", false, "v1").await;

    // The engine stamps the lifecycle when the command is sent.
    {
        let tx = begin(&env).await;
        let mut node = tx.get_node(TARGET_WS, &id).await.unwrap().unwrap();
        for (k, v) in [
            ("status", "sent"),
            ("attempt_id", "att-1"),
            ("sent_at", "2026-08-17T00:00:00Z"),
            ("sent_external_id", "CMD1"),
        ] {
            node.properties
                .insert(k.to_string(), PropertyValue::String(v.to_string()));
        }
        tx.upsert_node(TARGET_WS, &node).await.unwrap();
        tx.commit().await.unwrap();
    }

    // The provider now reports the object again, with a moved etag so the
    // upsert rebuilds rather than skipping. The mapper knows nothing about the
    // lifecycle and does not emit it.
    let mut index = mat.load_index(&scope).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert("unread".to_string(), Value::Bool(true));
    mat.apply_batch(
        &scope,
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: "CMD1.eml".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some("CMD1".to_string()),
                properties,
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: "CMD1".to_string(),
                etag: Some("v2".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();

    let node = node_by_id(&env, &id).await;
    // The rebuild really happened: the provider's new value landed.
    assert_eq!(
        node.properties.get("unread"),
        Some(&PropertyValue::Boolean(true)),
        "the upsert must have rebuilt the node, or this test proves nothing"
    );
    // ...and the lifecycle survived it.
    assert_eq!(
        str_prop(&node, "status").as_deref(),
        Some("sent"),
        "the re-sync erased the command lifecycle"
    );
    assert_eq!(str_prop(&node, "attempt_id").as_deref(), Some("att-1"));
    assert_eq!(str_prop(&node, "sent_external_id").as_deref(), Some("CMD1"));
    assert!(node.properties.contains_key("sent_at"));
}

/// ...but only for a type the mount declares as a command.
///
/// `status` is an ordinary provider field elsewhere — `stripe:Subscription` and
/// `stripe:PaymentIntent` both report one — so a blanket carry would freeze
/// those at whatever they were first synced as.
#[tokio::test(flavor = "multi_thread")]
async fn an_ordinary_node_still_takes_its_status_from_the_provider() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    // No `command_node_types`: an ordinary mount.
    let scope = watched_scope();
    let id = import_mail(&mat, &scope, "SUB1", false, "v1").await;

    {
        let tx = begin(&env).await;
        let mut node = tx.get_node(TARGET_WS, &id).await.unwrap().unwrap();
        node.properties
            .insert("status".to_string(), PropertyValue::String("active".into()));
        tx.upsert_node(TARGET_WS, &node).await.unwrap();
        tx.commit().await.unwrap();
    }

    let mut index = mat.load_index(&scope).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert("status".to_string(), Value::String("canceled".to_string()));
    mat.apply_batch(
        &scope,
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: "SUB1.eml".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some("SUB1".to_string()),
                properties,
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: "SUB1".to_string(),
                etag: Some("v2".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();

    assert_eq!(
        str_prop(&node_by_id(&env, &id).await, "status").as_deref(),
        Some("canceled"),
        "a provider status must keep tracking the provider"
    );
}
