//! What an adapter answering something other than "done" must NOT become.
//!
//! An `update` that returns `null` means the remote object is not there under
//! the id this mount recorded — the ms-graph adapter returns exactly that on a
//! 404, because Graph message ids are not stable (a message that moves folders
//! gets a new one) and treating a moved message as a broken mount would be
//! worse than useless. It is the one adapter reply that is neither a success
//! nor an error, and the drain used to read it as a success: `Value::Null` has
//! no `etag`, so the stored etag was carried forward and the node's LOCAL
//! values were stamped as `__pushed_state`. The edit never left the building,
//! `diverges()` answered false from then on, and the run reported
//! `pushed: 1, failed: 0`.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use serde_json::{json, Value};

use super::*;
/// The engine module under test. `super` is `tests` here, so engine items are
/// one level further out than they are in `tests.rs` itself.
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::MountState;
use sync::materializer::PUSHED_STATE_PROP;

fn state_only_mode() -> sync::write::WriteMode {
    sync::write::WriteMode::StateOnly(vec!["unread".to_string()])
}

fn maybe_pushed_state(node: &Node) -> Option<Value> {
    node.properties
        .get(PUSHED_STATE_PROP)
        .map(|pv| serde_json::to_value(pv).unwrap())
}

/// A `null` from `update` is not a push: nothing is stamped and nothing is
/// counted as pushed.
///
/// The scenario is the ordinary one for a mail mount: a message is marked read
/// in RaisinDB while, in Outlook, the same message is moved between folders and
/// takes a new Graph id. The drain PATCHes the id it has, Graph 404s, the
/// adapter answers `null`.
///
/// What must NOT happen is the baseline moving. `__pushed_state` is an
/// assertion about the PROVIDER, and after this exchange the provider has
/// nothing — recording the local value there is the same silent loss the seed
/// path was fixed to stop, arriving through the adapter instead.
#[tokio::test(flavor = "multi_thread")]
async fn a_gone_object_is_not_recorded_as_pushed() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    // The provider has nothing under this id any more.
    mock.set_update_reply(Value::Null);

    // Synced in read, with a baseline the sync itself recorded.
    let id = sync_in_mail(&mat, false, "v1").await;
    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": false }))
    );
    // The user marks it unread locally.
    set_bool_prop(&env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(mock.update_count(), 1, "the push was attempted");
    assert_eq!(
        stats.pushed, 0,
        "nothing reached the provider, so nothing may be counted as pushed"
    );
    assert_eq!(stats.gone, 1, "and the reason must be countable on its own");
    assert_eq!(stats.failed, 0);

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        maybe_pushed_state(&node),
        Some(json!({ "unread": false })),
        "the baseline still describes the last value the provider confirmed; \
         stamping the local edit here is exactly the silent loss"
    );
    assert_eq!(
        str_prop(&node, "__etag").as_deref(),
        Some("v1"),
        "and no etag was assigned, because no write happened"
    );

    // The operator is told, through the one field the drain writes.
    let reported = state
        .writeback_last_error
        .as_deref()
        .expect("an unsendable edit must not leave the mount looking clean");
    assert!(
        reported.contains("no longer has the object"),
        "the message must name what happened; got: {reported}"
    );
}

/// The edit stays pending, so a later drain tries again.
///
/// The corollary of not stamping. Under the old behaviour the second drain saw
/// a converged node and did nothing — the edit was settled without ever having
/// been sent. It must instead remain divergent: the id may be stale, but the
/// user's edit is still unsent, and the item will come back under its current
/// id on a later delta.
#[tokio::test(flavor = "multi_thread")]
async fn a_gone_object_leaves_the_edit_pending_for_the_next_drain() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    mock.set_update_reply(Value::Null);

    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();

    for expected in 1..=2 {
        let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
        let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
        assert_eq!(stats.gone, 1);
        assert_eq!(stats.pushed, 0);
        assert_eq!(
            mock.update_count(),
            expected,
            "an unsent edit must stay pending, not be settled by a 404"
        );
    }
}

/// A reply that is not an object at all is treated the same way.
///
/// The guard is `!result.is_object()`, not `== Value::Null`, on purpose: the
/// failure mode is `result.get("etag")` returning `None` and the stamp
/// proceeding on the stored etag, and every non-object reply — a bare string, a
/// number, an array — does that identically. An adapter answering nonsense must
/// not be able to fabricate a baseline.
#[tokio::test(flavor = "multi_thread")]
async fn a_non_object_reply_cannot_fabricate_a_baseline() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    mock.set_update_reply(json!("ok"));

    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.gone, 1);
    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": false }))
    );
}

/// An ordinary accepted update still stamps — the guard must not swallow the
/// success case it sits in front of.
#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_update_still_stamps_the_baseline() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();

    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let mount = state_only_mount();
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.gone, 0);
    assert_eq!(state.writeback_last_error, None);
    let node = node_by_id(&env, &id).await;
    assert_eq!(
        maybe_pushed_state(&node),
        Some(json!({ "unread": true })),
        "a confirmed write IS a baseline"
    );
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v2"));
}
