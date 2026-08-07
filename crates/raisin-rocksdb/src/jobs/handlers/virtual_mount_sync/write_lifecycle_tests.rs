//! Lifecycle of the writeback drain: the lease it must keep, the budget it must
//! respect, the stop it must honour, and the modes it must refuse out loud.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers while
//! keeping it from growing further.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use raisin_error::Result as RaisinResult;
use raisin_locks::{FencingToken, LockGuard, LockManager, LockManagerHandle};
use raisin_models::nodes::properties::PropertyValue;
use serde_json::{json, Value};

use super::*;
/// The engine module under test. `super` is `tests` here, so engine items are
/// one level further out than they are in `tests.rs` itself.
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{MappedNode, MountState, WriteConfig};
use sync::materializer::{BatchOp, MountScope, VirtualMeta};
use sync::{AdapterError, AdapterInvoker, MapperWriteback, SyncCtx};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// A lock manager that only counts. The drain's renewal has to be observable
/// somewhere, and `renew_lease` is a silent no-op unless BOTH a manager and a
/// lease token are present — which is exactly why the missing renewal could ship
/// unnoticed.
#[derive(Default)]
struct CountingLocks {
    renewals: AtomicUsize,
}

impl CountingLocks {
    fn count(&self) -> usize {
        self.renewals.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LockManager for CountingLocks {
    async fn try_acquire(
        &self,
        key: &str,
        _owner: &str,
        _ttl: Duration,
    ) -> RaisinResult<Option<LockGuard>> {
        Ok(Some(LockGuard {
            key: key.to_string(),
            token: 1,
            expires_at_ms: 0,
        }))
    }
    async fn release(&self, _key: &str, _token: FencingToken) -> RaisinResult<bool> {
        Ok(true)
    }
    async fn renew(&self, _key: &str, _token: FencingToken, _ttl: Duration) -> RaisinResult<bool> {
        self.renewals.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
    async fn claim(&self, _pool: &str, _n: u64, _capacity: u64) -> RaisinResult<Option<u64>> {
        Ok(Some(0))
    }
    async fn release_claim(&self, _pool: &str, _n: u64) -> RaisinResult<u64> {
        Ok(0)
    }
}

/// The `state_only` mode the drain runs under in these tests.
fn state_only_mode() -> sync::write::WriteMode {
    sync::write::WriteMode::StateOnly(sync::write::FieldPlan::pushing(&["unread"]))
}

/// Materialize `n` mail-shaped nodes in ONE batch, seeded (`__pushed_state`
/// present) so each is a push candidate rather than a seed candidate.
async fn sync_in_mails(mat: &RocksDbMaterializer, n: usize) -> Vec<String> {
    let mut index = mat.load_index(&watched_scope()).await.unwrap();
    let ops: Vec<BatchOp> = (0..n)
        .map(|i| {
            let mut properties = serde_json::Map::new();
            properties.insert("unread".to_string(), Value::Bool(false));
            BatchOp::Upsert {
                rel_path: format!("m{i:02}.eml"),
                mapped: MappedNode {
                    node_type: "raisin:Node".to_string(),
                    name: Some(format!("m{i:02}")),
                    properties,
                },
                virt: VirtualMeta {
                    mount_id: MOUNT_ID.to_string(),
                    external_id: format!("M{i:02}"),
                    etag: Some("v1".to_string()),
                    synced_at: Utc::now().to_rfc3339(),
                },
            }
        })
        .collect();
    mat.apply_batch(&watched_scope(), &mut index, ops)
        .await
        .unwrap();
    let mut ids: Vec<(String, String)> = index
        .virtual_nodes()
        .into_iter()
        .map(|n| (n.external_id, n.id))
        .collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

/// The local edit that makes every one of them diverge, in one transaction.
async fn flip_unread(env: &Env, ids: &[String]) {
    let tx = begin(env).await;
    for id in ids {
        let mut node = tx.get_node(TARGET_WS, id).await.unwrap().unwrap();
        node.properties
            .insert("unread".to_string(), PropertyValue::Boolean(true));
        tx.upsert_node(TARGET_WS, &node).await.unwrap();
    }
    tx.commit().await.unwrap();
}

/// How many of these nodes now record `unread: true` as pushed — i.e. how many
/// stamps actually reached storage.
async fn stamped_count(env: &Env, ids: &[String]) -> usize {
    let mut n = 0;
    for id in ids {
        let node = node_by_id(env, id).await;
        if pushed_state(&node) == json!({"unread": true}) {
            n += 1;
        }
    }
    n
}

/// Ask the drain to stop, the way the Stop endpoint does: through the mount's
/// persisted state on the config branch.
async fn request_stop(env: &Env) {
    let mut state = sync::read_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID)
        .await
        .unwrap()
        .unwrap_or_default();
    state.stop_requested = true;
    assert!(
        sync::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut state)
            .await
            .unwrap(),
        "the stop flag must actually land"
    );
}

// ---------------------------------------------------------------------------
// 1. lease renewal
// ---------------------------------------------------------------------------

/// The drain renews the mount lease as it works.
///
/// Without this the lease expires mid-drain on any mount with more than a
/// handful of edits behind a throttled provider: a second run then starts beside
/// this one and both write `__etag` for the same items, which is the one way a
/// remote change is lost. The read phases have renewed per page since they were
/// written; the drain shipped with no renewal at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_drain_renews_the_lease_as_it_pushes() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let ids = sync_in_mails(&mat, 3).await;
    flip_unread(&env, &ids).await;

    let locks = Arc::new(CountingLocks::default());
    let handle: LockManagerHandle = locks.clone();
    let c = SyncCtx {
        lock_manager: Some(handle),
        lease_token: Some(7),
        ..ctx(&env, &mount, &mock, &mat)
    };
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.pushed, 3);
    assert_eq!(mock.update_count(), 3);
    assert_eq!(
        locks.count(),
        3,
        "one renewal per item pushed: a single throttled `update` can outlast a \
         page-boundary renewal interval"
    );
}

// ---------------------------------------------------------------------------
// 2. the wall-clock budget
// ---------------------------------------------------------------------------

/// A drain with no budget left stops before starting another provider call, and
/// leaves every unpushed edit pending.
///
/// The failure it prevents is not a slow drain: it is the watchdog reaping the
/// run mid-`update`, which skips `finalize` entirely — status wedged at
/// `"syncing"`, lease leaked for its full TTL. Both read phases stop themselves
/// for exactly this reason; the drain runs FIRST and could spend the whole
/// budget before either of them was reached.
#[tokio::test(flavor = "multi_thread")]
async fn a_drain_out_of_time_stops_and_leaves_the_edits_pending() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let ids = sync_in_mails(&mat, 2).await;
    flip_unread(&env, &ids).await;

    let spent = SyncCtx {
        deadline: Utc::now().timestamp() - 1,
        ..ctx(&env, &mount, &mock, &mat)
    };
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&spent).await.unwrap();
    let stats = sync::write::drain(&spent, &mut state, &mut batcher, &state_only_mode()).await;

    assert!(
        stats.truncated,
        "the truncation must be recorded, not silent"
    );
    assert_eq!(stats.pushed, 0);
    assert_eq!(mock.update_count(), 0, "no call may START without budget");
    assert_eq!(
        stamped_count(&env, &ids).await,
        0,
        "an unpushed edit must never be stamped as pushed"
    );

    // Pending, not lost: the next run has a fresh budget and drains them.
    let fresh = ctx(&env, &mount, &mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&fresh).await.unwrap();
    let stats = sync::write::drain(&fresh, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats.pushed, 2);
    assert!(!stats.truncated);
    assert_eq!(stamped_count(&env, &ids).await, 2);
}

// ---------------------------------------------------------------------------
// 3. cooperative stop
// ---------------------------------------------------------------------------

/// An operator's Stop ends the drain, and what was already pushed is still
/// flushed.
///
/// Stop was honoured by both read phases and by nothing on the write side, so
/// the externally-visible half of a run — the half that changes other people's
/// mailboxes — kept going after the operator had asked it to stop.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_ends_the_drain_at_the_next_checkpoint() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let ids = sync_in_mails(&mat, 3).await;
    flip_unread(&env, &ids).await;
    request_stop(&env).await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert!(stats.stopped, "the stop must be recorded on the drain");
    assert_eq!(
        mock.update_count(),
        0,
        "the check runs on the FIRST iteration too, or a small drain pushes its \
         whole candidate list before noticing"
    );
    assert_eq!(stamped_count(&env, &ids).await, 0);

    // Clearing the stop lets the very same edits drain normally — they were
    // parked, not consumed.
    //
    // `run_sync` now does this clearing itself, once a run has reached its end:
    // a stop is a request to end THIS run, not a mode the mount stays in.
    // Nothing else in the engine ever cleared the flag, so a mount that was
    // stopped once stayed stopped — every later run exited at its first
    // checkpoint and reported a cheerful `ok` with nothing done, while
    // scheduling carried on as normal. That is what this manual clear stands in
    // for; the automatic one lives at the end of `run_sync`.
    let mut cleared = sync::read_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID)
        .await
        .unwrap()
        .unwrap();
    cleared.stop_requested = false;
    sync::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut cleared)
        .await
        .unwrap();

    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert!(!stats.stopped);
    assert_eq!(stats.pushed, 3);
    assert_eq!(stamped_count(&env, &ids).await, 3);
}

/// A stop landing PART-WAY through keeps everything already pushed: the drain
/// breaks to its flush rather than returning past it, so the stamps for the
/// items it did send reach storage ahead of the read phases.
#[tokio::test(flavor = "multi_thread")]
async fn a_mid_drain_stop_still_flushes_what_was_pushed() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    // 25 candidates: the checkpoints fall at item 0 and item 20, so a stop
    // raised during the fifth push is seen at 20 with 20 items already sent.
    let ids = sync_in_mails(&mat, 25).await;
    flip_unread(&env, &ids).await;

    let mock = StoppingMock {
        inner: StateOnlyMock::new(),
        storage: env.storage.clone(),
        stop_after: 5,
    };
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert!(stats.stopped);
    assert_eq!(
        stats.pushed, 20,
        "stopped at the checkpoint, not at the edit"
    );
    assert_eq!(
        stamped_count(&env, &ids).await,
        20,
        "the flush must still run after an early break"
    );
}

/// [`StateOnlyMock`] that raises the mount's stop flag part-way through the
/// drain — the operator hitting Stop while pushes are in flight.
struct StoppingMock {
    inner: StateOnlyMock,
    storage: Arc<crate::RocksDBStorage>,
    stop_after: usize,
}

#[async_trait]
impl AdapterInvoker for StoppingMock {
    async fn invoke(
        &self,
        scope: &MountScope,
        path: &str,
        input: Value,
    ) -> std::result::Result<Value, AdapterError> {
        let out = self.inner.invoke(scope, path, input.clone()).await;
        if input.get("operation").and_then(|v| v.as_str()) == Some("update")
            && self.inner.update_count() == self.stop_after
        {
            let mut state = sync::read_mount_state(&self.storage, TENANT, REPO, "main", MOUNT_ID)
                .await
                .unwrap()
                .unwrap_or_default();
            state.stop_requested = true;
            sync::persist_mount_state(&self.storage, TENANT, REPO, "main", MOUNT_ID, &mut state)
                .await
                .unwrap();
        }
        out
    }
}

// ---------------------------------------------------------------------------
// 4. modes the engine cannot honour
// ---------------------------------------------------------------------------

/// A documented-but-unimplemented mode, and a typo, are both REFUSED with a
/// reason rather than quietly demoted to `off`.
///
/// `submit` is in `docs/reference/virtual-node-adapters.md` §10.2, so an
/// operator can configure it by following the documentation. That used to
/// produce a mount that wrote nothing, reported `writeback_supported: null`
/// ("not applicable") and logged not one line — indistinguishable from a
/// read-only mount. A misspelled mode behaved identically.
///
/// `mirror` was in this list until Stage 7 implemented it; it is now covered by
/// `write_mirror_tests`, and the case that matters here is the one that
/// replaced it — a mirror whose ADAPTER falls short is still refused, with the
/// missing ops named.
#[test]
fn an_unimplemented_or_unknown_write_mode_is_refused_loudly() {
    let caps = sync::Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        can_create: true,
        can_delete: true,
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    let wc = |mode: &str| WriteConfig {
        mode: mode.to_string(),
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    let reason =
        |mode: &str| match sync::write::resolve_mode(&wc(mode), &caps, &MapperWriteback::Supported)
        {
            sync::write::WriteMode::Refused(r) => r,
            other => panic!("expected `{mode}` to be refused, got {other:?}"),
        };

    // `submit` was in this list until Stage 10 implemented it. It is now
    // refused for the HONEST reason instead — this adapter declares create,
    // update and delete and no `can_submit`, and a mount is never more able to
    // send than its adapter says it is.
    for mode in ["submit", "SUBMIT"] {
        let r = reason(mode);
        assert!(r.contains("can_submit"), "{r}");
        assert!(
            !r.contains("not implemented"),
            "the engine implements submit; refusing it as unimplemented would send \
             an operator to fix the wrong thing: {r}"
        );
    }
    let typo = reason("state-only");
    assert!(typo.contains("not recognized"), "{typo}");

    // `mirror` is implemented and resolves against these full capabilities.
    assert!(matches!(
        sync::write::resolve_mode(&wc("mirror"), &caps, &MapperWriteback::Supported),
        sync::write::WriteMode::Mirror(_)
    ));

    // The verdict the console reads is the SAME decision, not a second copy of
    // it: `(Some(false), reason)`, never the `(None, None)` that means
    // "writeback not applicable".
    let (supported, verdict) =
        sync::write::writeback_verdict(&wc("submit"), &caps, &MapperWriteback::Supported);
    assert_eq!(supported, Some(false));
    assert!(verdict.unwrap().contains("can_submit"));

    // `off` (and an absent mode) stay silent — a read-only mount must not start
    // reporting a refusal it never asked for.
    assert_eq!(
        sync::write::resolve_mode(&wc("off"), &caps, &MapperWriteback::Supported),
        sync::write::WriteMode::Off
    );
    assert_eq!(
        sync::write::writeback_verdict(&wc("off"), &caps, &MapperWriteback::Supported),
        (None, None)
    );
    assert_eq!(
        sync::write::writeback_verdict(&WriteConfig::default(), &caps, &MapperWriteback::NoMapper),
        (None, None)
    );
}

/// A refused mount does not push, whatever its nodes look like.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_mode_pushes_nothing() {
    let env = setup().await;
    let mut mount = state_only_mount();
    mount.write_config.mode = "mirror".to_string();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let ids = sync_in_mails(&mat, 1).await;
    flip_unread(&env, &ids).await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let caps = sync::Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    let mode = sync::write::resolve_mode(&mount.write_config, &caps, &MapperWriteback::Supported);
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &mode).await;

    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0);
}

// ---------------------------------------------------------------------------
// 5. the drain leaves a receipt
// ---------------------------------------------------------------------------

/// A half-drained mount must not look like an idle one.
///
/// `DrainStats` lives and dies inside `drain`; `phases.rs` drops the return
/// value. So before `last_drain`, a run that pushed 0 of 2 edits and stopped on
/// its budget produced exactly the same observable output as a run with nothing
/// to do — `outcome: "ok"`, every batcher counter zero, no state field. An
/// operator had no way to tell a mount that is falling behind from one that is
/// caught up, which is the whole reason `truncated` and `stopped` are recorded
/// rather than inferred.
#[tokio::test(flavor = "multi_thread")]
async fn a_truncated_drain_is_distinguishable_from_an_idle_one() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let ids = sync_in_mails(&mat, 2).await;
    flip_unread(&env, &ids).await;

    // Out of budget with both edits still pending.
    let spent = SyncCtx {
        deadline: Utc::now().timestamp() - 1,
        ..ctx(&env, &mount, &mock, &mat)
    };
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&spent).await.unwrap();
    sync::write::drain(&spent, &mut state, &mut batcher, &state_only_mode()).await;

    let truncated = state
        .last_drain
        .clone()
        .expect("a drain that had candidates must leave a receipt");
    assert!(truncated.truncated);
    assert_eq!(truncated.pushed, 0);
    assert_eq!(
        truncated.pending, 2,
        "the queue depth is the only thing that says this mount is behind"
    );

    // Same mount, fresh budget: the receipt must now say caught up, not keep
    // reporting the old backlog.
    let fresh = ctx(&env, &mount, &mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&fresh).await.unwrap();
    sync::write::drain(&fresh, &mut state, &mut batcher, &state_only_mode()).await;

    let caught_up = state
        .last_drain
        .clone()
        .expect("a drain that pushed must leave a receipt");
    assert_eq!(caught_up.pushed, 2);
    assert_eq!(
        caught_up.pending, 0,
        "a stale backlog would read as a mount still behind"
    );
    assert!(!caught_up.truncated);
    assert!(!caught_up.stopped);
}

/// A drain with no candidates leaves the previous receipt alone.
///
/// The counterpart to the assertion above: `last_drain` is written on every
/// drain that had work, so "nothing to do" must not overwrite it with zeroes —
/// that would erase the record of the run that actually pushed.
#[tokio::test(flavor = "multi_thread")]
async fn an_idle_drain_writes_no_receipt() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    sync_in_mails(&mat, 2).await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats, Default::default());
    assert!(
        state.last_drain.is_none(),
        "an idle drain must not stamp a receipt over a real one"
    );
}

/// A push that fails with `config_error` ENDS the drain and stands the mount
/// off, instead of leaving the edit pending for the next run to retry.
///
/// This shipped broken and reached production. A missing `Calendars.ReadWrite`
/// scope makes every push 403 forever, and the drain logged "leaving the edit
/// pending" and moved on to the next candidate — so the same un-pushable edits
/// were retried on every drain, permanently. It burned a core and spent the
/// provider's rate limit on calls that were always going to be refused.
///
/// Three things are asserted, because fixing only the first would still loop:
/// the drain STOPS rather than working through the remaining candidates, the
/// mount is badged so an operator can see why, and a stand-off is recorded so
/// the next run does not immediately try again.
#[tokio::test(flavor = "multi_thread")]
async fn a_config_error_ends_the_drain_and_stands_the_mount_off() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let ids = sync_in_mails(&mat, 3).await;
    flip_unread(&env, &ids).await;
    mock.fail_updates_with_config_error("missing Calendars.ReadWrite");

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let before = Utc::now().timestamp();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert!(stats.misconfigured, "a config error must be terminal");
    assert_eq!(
        mock.update_count(),
        1,
        "the drain must stop at the FIRST config error, not try all three: every \
         remaining push would fail identically"
    );
    assert_eq!(
        state.writeback_status.as_deref(),
        Some("misconfigured"),
        "the operator has to be able to see why nothing is being pushed"
    );
    let retry_after = state
        .writeback_retry_after
        .expect("a stand-off must be recorded, or the next drain retries at once");
    assert!(
        retry_after > before,
        "the stand-off must be in the future: {retry_after} vs {before}"
    );

    // And the stand-off is honoured: a second drain makes no provider call.
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let again = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(again.pushed, 0);
    assert_eq!(
        mock.update_count(),
        1,
        "the second drain must make NO call while the stand-off is open — that is \
         the whole point of it"
    );
}

/// A run that was killed before it could finalize is closed out by the next
/// one, instead of sitting at `running` forever.
///
/// The watchdog drops a sync at its current await point, so `finalize` never
/// executes: `status` stays `"syncing"` and the newest run record stays
/// `running`, and nothing else in the system rewrites either. The console then
/// shows a mount permanently "syncing now" with `Sync now` greyed out —
/// busy-looking and doing nothing — which is exactly what a throttled calendar
/// mount looked like in production.
#[test]
fn a_killed_run_is_closed_out_by_the_next_one() {
    let mut state = MountState::default();
    let started = Utc::now().timestamp() - 3600;
    state.status = Some("syncing".to_string());
    state.last_run = Some(sync::config::SyncRun::started(started, "delta", "schedule"));

    let ran_for = sync::run::close_interrupted_run(&mut state, started + 3600)
        .expect("an open run must be closed");
    assert_eq!(ran_for, 3600);

    let closed = state.last_run.as_ref().expect("the record survives");
    assert_eq!(
        closed.outcome, "interrupted",
        "a killed run is not an error and not a success — it has its own outcome"
    );
    assert!(
        closed.finished_at.is_some(),
        "an open run with no end time reads as still running"
    );
    assert_eq!(
        state.recent_runs.first().map(|r| r.outcome.as_str()),
        Some("interrupted"),
        "it must also reach the history, or the console shows no trace of the kill"
    );

    // A run that ended properly is left exactly as it is.
    let mut settled = MountState::default();
    settled.last_run = Some({
        let mut r = sync::config::SyncRun::started(started, "delta", "schedule");
        r.finish(started + 5, "ok", None);
        r
    });
    assert!(
        sync::run::close_interrupted_run(&mut settled, started + 3600).is_none(),
        "only an OPEN run is closed; rewriting a finished one would fabricate history"
    );
}
