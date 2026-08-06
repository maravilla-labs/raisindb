//! The latency drain (`VirtualMountWriteDrain`) as a job, and the promise that
//! nothing depends on it.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use raisin_models::nodes::Node;
use raisin_storage::jobs::JobType;
use serde_json::{json, Value};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;

/// Turn the persisted mount node into the stage-2 slice: `state_only` over one
/// boolean field, with a mapper that can reverse-map.
///
/// The in-memory [`state_only_mount`] cannot be used here — these tests go
/// through the JOB, which reads the mount from storage.
async fn make_mount_writable(env: &Env, state: Value) {
    let tx = begin(env).await;
    let mut node = tx
        .get_node(sync::SYSTEM_WORKSPACE, MOUNT_ID)
        .await
        .unwrap()
        .unwrap();
    node.properties.insert(
        "mapping_function".to_string(),
        PropertyValue::String("/mappers/mail".to_string()),
    );
    node.properties.insert(
        "write_config".to_string(),
        prop_obj(json!({ "mode": "state_only", "mutable_fields": ["unread"] })),
    );
    node.properties.insert("state".to_string(), prop_obj(state));
    tx.upsert_node(sync::SYSTEM_WORKSPACE, &node).await.unwrap();
    tx.commit().await.unwrap();
}

async fn mount_node(env: &Env) -> Node {
    let tx = begin(env).await;
    tx.get_node(sync::SYSTEM_WORKSPACE, MOUNT_ID)
        .await
        .unwrap()
        .unwrap()
}

/// One diverging mail node, materialized the way a sync would and then edited
/// the way a user would.
async fn a_pending_local_edit(env: &Env) -> String {
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(env, &id, "unread", true).await;
    id
}

fn drain_job(trigger: &str) -> JobInfo {
    job_info(JobType::VirtualMountWriteDrain {
        mount_id: MOUNT_ID.to_string(),
        trigger: trigger.to_string(),
    })
}

// ---------------------------------------------------------------------------
// 1. the drain job pushes, and reads nothing
// ---------------------------------------------------------------------------

/// The latency job pushes the pending edit and makes no read call at all.
///
/// Reading is the expensive half and the half the schedule already owns. A
/// drain that also ran a delta would turn every local edit into a provider
/// round trip on top of the mount's own polling.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_drain_job_pushes_and_reads_nothing() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    make_mount_writable(&env, json!({ "last_sync_token": "tok-1" })).await;
    let id = a_pending_local_edit(&env).await;

    let mock = Arc::new(StateOnlyMock::new());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as sync::AdapterInvokerHandle),
        None,
    );
    handler
        .handle(&drain_job("capture"), &job_context())
        .await
        .expect("a drain must never fail the job");

    assert_eq!(mock.update_count(), 1, "the pending edit must be pushed");
    assert_eq!(
        pushed_state(&node_by_id(&env, &id).await),
        json!({"unread": true}),
        "and baselined, or the next drain pushes it again"
    );
    let calls = mock.calls.lock().unwrap().clone();
    assert!(
        !calls.iter().any(|c| c == "get_changes" || c == "list"),
        "a latency drain must not read the provider; calls were {calls:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. the drain must not postpone the read schedule
// ---------------------------------------------------------------------------

/// A drain leaves the READ scheduler exactly where it found it.
///
/// This is the sharpest edge in routing the drain through `run_sync`.
/// `check::is_due` schedules against `max(last_sync_at, last_attempt_at)`, and
/// a capture-driven drain fires on every local edit — so stamping either would
/// let a mailbox that is being worked through continuously postpone its own
/// delta poll indefinitely, one edit at a time. Remote changes would stop
/// arriving while the mount reported `ok` with a fresh run in its history:
/// silent, and attributable to nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_drain_does_not_postpone_the_next_read() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    // A mount that has read successfully once, long ago, and is due now.
    let last_read = 1_000_000i64;
    make_mount_writable(
        &env,
        json!({ "last_sync_token": "tok-1", "last_sync_at": last_read, "status": "ok" }),
    )
    .await;
    a_pending_local_edit(&env).await;

    let mock = Arc::new(StateOnlyMock::new());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as sync::AdapterInvokerHandle),
        None,
    );
    handler
        .handle(&drain_job("capture"), &job_context())
        .await
        .unwrap();
    assert_eq!(mock.update_count(), 1, "the drain must have done its work");

    let state = read_state(&env).await.expect("state must be persisted");
    assert_eq!(
        state.get("last_sync_at").and_then(|v| v.as_i64()),
        Some(last_read),
        "a drain reads nothing, so it may not claim a successful read"
    );
    assert_eq!(
        state.get("last_attempt_at").and_then(|v| v.as_i64()),
        None,
        "nor may it start the read backoff clock"
    );

    // The observable consequence, stated as the scheduler sees it.
    let mount = MountConfig::from_node(&mount_node(&env).await).unwrap();
    assert!(
        sync::check::is_due(&mount, last_read + 300),
        "the mount must still be due for its delta poll after a drain"
    );
    // And the drain's own receipt IS recorded, so the run is not invisible.
    assert_eq!(
        state
            .get("last_drain")
            .and_then(|d| d.get("pushed"))
            .and_then(|v| v.as_u64()),
        Some(1)
    );
}

/// A drain leaves `status` alone, too.
///
/// `status` is the READ side's verdict — the console greys out `Sync now` on
/// `"syncing"` and `check::is_due` refuses to schedule `"auth_required"`. A
/// successful push says nothing about whether the changes feed is healthy, so
/// writing `"ok"` here would clear a `degraded` mount because someone marked a
/// mail as read.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_drain_does_not_overwrite_the_read_status() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    make_mount_writable(
        &env,
        json!({
            "last_sync_token": "tok-1",
            "status": "degraded",
            "consecutive_failures": 4,
            "last_error": "provider hiccup",
        }),
    )
    .await;
    a_pending_local_edit(&env).await;

    let mock = Arc::new(StateOnlyMock::new());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as sync::AdapterInvokerHandle),
        None,
    );
    handler
        .handle(&drain_job("capture"), &job_context())
        .await
        .unwrap();

    let state = read_state(&env).await.unwrap();
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("degraded")
    );
    assert_eq!(
        state.get("consecutive_failures").and_then(|v| v.as_u64()),
        Some(4)
    );
    assert_eq!(
        state.get("last_error").and_then(|v| v.as_str()),
        Some("provider hiccup")
    );
}

// ---------------------------------------------------------------------------
// 3. correctness does not depend on the capture hook
// ---------------------------------------------------------------------------

/// With NO capture hook anywhere in the loop, an ordinary scheduled sync still
/// pushes the pending edit and converges it.
///
/// This is the whole claim of the latency stage: the hook is an optimization.
/// Nothing here publishes a node event, and no `VirtualMountWriteDrain` is ever
/// enqueued — the edit goes out because the drain is the first phase of every
/// run, which is where correctness lives.
#[tokio::test(flavor = "multi_thread")]
async fn a_scheduled_sync_converges_with_no_capture_hook() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    make_mount_writable(&env, json!({ "last_sync_token": "tok-1" })).await;
    let id = a_pending_local_edit(&env).await;

    let mock = Arc::new(StateOnlyMock::new());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as sync::AdapterInvokerHandle),
        None,
    );
    handler
        .handle(
            &job_info(JobType::VirtualMountSync {
                mount_id: MOUNT_ID.to_string(),
                mode: "delta".to_string(),
                trigger: "schedule".to_string(),
            }),
            &job_context(),
        )
        .await
        .unwrap();

    assert_eq!(mock.update_count(), 1, "the drain runs ahead of the read");
    assert_eq!(
        pushed_state(&node_by_id(&env, &id).await),
        json!({"unread": true})
    );
    let calls = mock.calls.lock().unwrap().clone();
    assert!(
        calls.iter().any(|c| c == "get_changes"),
        "...and the read phase still ran: {calls:?}"
    );

    // A second run pushes nothing: the converge check sees the baseline. This
    // is what makes a noisy capture layer safe to add on top.
    handler
        .handle(
            &job_info(JobType::VirtualMountSync {
                mount_id: MOUNT_ID.to_string(),
                mode: "delta".to_string(),
                trigger: "schedule".to_string(),
            }),
            &job_context(),
        )
        .await
        .unwrap();
    assert_eq!(mock.update_count(), 1, "a converged edit must not re-push");

    // ...and this one DID read, so it stamps the read clocks.
    let state = read_state(&env).await.unwrap();
    assert!(state.get("last_sync_at").and_then(|v| v.as_i64()).is_some());
    assert_eq!(state.get("status").and_then(|v| v.as_str()), Some("ok"));
}
