//! A mount config that does not parse must SAY so, in the place an operator
//! looks.
//!
//! Refusing to default a corrupt blob (rather than silently disabling
//! writeback, or silently resetting a cursor) is only half the guarantee. The
//! other half is that the refusal reaches somebody: before this, both callers
//! of [`MountConfig::from_node`](super::config::MountConfig::from_node)
//! swallowed it — the periodic scan logged a `warn` and moved on, and the job
//! path returned a validation error that wrote no mount state at all. The mount
//! stopped syncing in BOTH directions while the console kept showing the last
//! successful run, `status: "ok"`, a frozen `last_attempt_at` and no error.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use serde_json::json;

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::MountState;

/// Replace one property on the mount config node.
async fn set_mount_prop(env: &Env, key: &str, value: serde_json::Value) {
    let tx = begin(env).await;
    let mut node = tx
        .get_node(sync::SYSTEM_WORKSPACE, MOUNT_ID)
        .await
        .unwrap()
        .unwrap();
    node.properties.insert(key.to_string(), prop_obj(value));
    tx.upsert_node(sync::SYSTEM_WORKSPACE, &node).await.unwrap();
    tx.commit().await.unwrap();
}

/// `mutable_fields` as a string where a list belongs — the shape an operator or
/// a shipped package actually gets wrong.
fn corrupt_write_config() -> serde_json::Value {
    json!({ "mode": "state_only", "mutable_fields": "unread" })
}

/// The job path records the verdict instead of failing invisibly.
///
/// It used to `return Err(Error::Validation(..))`, which fails the job and
/// writes nothing: `state.status` kept whatever the last successful run left
/// there, `last_attempt_at` froze, and `writeback_last_error` stayed empty.
#[tokio::test(flavor = "multi_thread")]
async fn an_unparseable_mount_is_marked_misconfigured_by_the_job() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    // A run that succeeded, so `status: "ok"` is what the corruption has to
    // displace — exactly the stale-green the old path left behind.
    let mut ok_state = MountState {
        status: Some("ok".to_string()),
        last_sync_token: Some("cursor-42".to_string()),
        ..Default::default()
    };
    sync::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut ok_state)
        .await
        .unwrap();

    set_mount_prop(&env, "write_config", corrupt_write_config()).await;

    let mock = Arc::new(MockAdapter::default());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as sync::AdapterInvokerHandle),
        None,
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "delta".to_string(),
        trigger: "manual".to_string(),
    });
    let result = handler
        .handle(&job, &job_context())
        .await
        .expect("a corrupt config is a mount problem, not a job crash");

    assert_eq!(
        result
            .as_ref()
            .and_then(|v| v.get("reason"))
            .and_then(|v| v.as_str()),
        Some("misconfigured"),
        "the run must name why it did not happen"
    );

    let state = read_state(&env).await.expect("state must be persisted");
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("misconfigured"),
        "the console reads this field; leaving it at `ok` is the whole defect"
    );
    let err = state
        .get("last_error")
        .and_then(|v| v.as_str())
        .expect("the verdict must carry its reason");
    assert!(
        err.contains("write_config"),
        "and name the property to fix; got: {err}"
    );
    assert!(
        state
            .get("last_attempt_at")
            .and_then(|v| v.as_i64())
            .is_some(),
        "the attempt must be stamped, like every other misconfigured exit"
    );
    assert_eq!(
        state.get("last_sync_token").and_then(|v| v.as_str()),
        Some("cursor-42"),
        "marking the mount must not cost it its cursor: the corruption is in \
         another property and the state blob is merged, not rebuilt"
    );
}

/// The periodic scan records it too — and does not enqueue.
///
/// The scan is the path that runs every 60s whether or not anyone presses
/// anything, so it is where an operator's first evidence has to come from. It
/// used to emit one `warn` (production runs at `RUST_LOG=warn`) and `continue`.
#[tokio::test(flavor = "multi_thread")]
async fn the_periodic_scan_marks_an_unparseable_mount() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    set_mount_prop(&env, "write_config", corrupt_write_config()).await;

    let enqueued = sync::check::run_check(
        &env.storage,
        Some(TENANT.to_string()),
        Some(REPO.to_string()),
    )
    .await
    .unwrap();
    assert_eq!(enqueued, 0, "an unparseable mount cannot be synced");

    let state = read_state(&env).await.expect("state must be persisted");
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("misconfigured")
    );
    assert!(state
        .get("last_error")
        .and_then(|v| v.as_str())
        .is_some_and(|e| e.contains("write_config")));
}

/// Marking is idempotent, so the 60s scan does not mint a revision per tick.
///
/// Load-bearing rather than tidy: this path is reached on every scan for as
/// long as the mount stays broken, the mount node is replicated, and an
/// unconditional write would produce a revision a minute, forever — trading one
/// silent failure for a noisy one.
#[tokio::test(flavor = "multi_thread")]
async fn marking_the_same_verdict_twice_writes_once() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    set_mount_prop(&env, "write_config", corrupt_write_config()).await;

    let first = sync::misconfig::mark_unparseable_mount(
        &env.storage,
        TENANT,
        REPO,
        "main",
        MOUNT_ID,
        "invalid mount: write_config: ...",
    )
    .await
    .unwrap();
    assert!(first, "the first verdict must land");

    let second = sync::misconfig::mark_unparseable_mount(
        &env.storage,
        TENANT,
        REPO,
        "main",
        MOUNT_ID,
        "invalid mount: write_config: ...",
    )
    .await
    .unwrap();
    assert!(!second, "the same verdict must not rewrite the node");

    // A DIFFERENT reason still gets through — the guard is on the verdict, not
    // on the status.
    let changed = sync::misconfig::mark_unparseable_mount(
        &env.storage,
        TENANT,
        REPO,
        "main",
        MOUNT_ID,
        "invalid mount: state: ...",
    )
    .await
    .unwrap();
    assert!(changed);
}

/// A `state` blob that is not even an object is set aside, never overwritten.
///
/// The reason `parse_object_checked` refuses to default a corrupt blob is that
/// silently replacing it loses the cursor. Writing the verdict must not do
/// under a different name what that refusal exists to prevent, so the original
/// value is kept verbatim beside the marker.
#[tokio::test(flavor = "multi_thread")]
async fn a_non_object_state_is_preserved_beside_the_verdict() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    set_mount_prop(&env, "state", json!("totally-not-a-state")).await;

    sync::misconfig::mark_unparseable_mount(
        &env.storage,
        TENANT,
        REPO,
        "main",
        MOUNT_ID,
        "invalid mount: state: ...",
    )
    .await
    .unwrap();

    let state = read_state(&env).await.expect("state must be persisted");
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("misconfigured")
    );
    assert_eq!(
        state.get("state_unparseable").and_then(|v| v.as_str()),
        Some("totally-not-a-state"),
        "the unreadable value is evidence; dropping it is not our call"
    );
}
