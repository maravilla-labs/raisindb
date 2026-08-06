//! What happens when the provider REFUSES a push.
//!
//! A conflict is the one adapter answer whose meaning the engine cannot decide:
//! the object changed since this mount read it, and whether the local edit or
//! the remote one matters is a property of the data, not of the engine. Before
//! this module the answer was hardcoded — a conflict counted as a failure, the
//! edit stayed pending, and `write_config.conflict` was parsed and never read.
//!
//! The four policies differ in exactly one place (what the drain does after the
//! refusal) and in what they cost when they are wrong, which is why the default
//! is the recoverable one. `remote_wins` loses a local edit the user can redo;
//! `local_wins` overwrites another writer's change unrecoverably.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::*;
/// The engine module under test. `super` is `tests` here, so engine items are
/// one level further out than they are in `tests.rs` itself.
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::MountState;
use sync::materializer::{MountScope, PUSHED_STATE_PROP};
use sync::write::ConflictPolicy;
use sync::{AdapterError, AdapterInvoker};

fn state_only_mode() -> sync::write::WriteMode {
    sync::write::WriteMode::StateOnly(sync::write::FieldPlan::pushing(&["unread"]))
}

fn maybe_pushed_state(node: &Node) -> Option<Value> {
    node.properties
        .get(PUSHED_STATE_PROP)
        .map(|pv| serde_json::to_value(pv).unwrap())
}

/// A mount whose provider refuses the first `update` of every item, plus a
/// resolver whose answer the test chooses.
///
/// The refusal is FIRST-ONLY on purpose: a mock that refused forever could not
/// distinguish "the engine gave up" from "the engine retried unconditionally and
/// the provider still said no", and the second attempt carrying `etag: null` is
/// the entire mechanism of `local_wins`.
struct ConflictMock {
    /// `params` of every `update`, in order — so a test can assert what the
    /// second attempt carried.
    updates: Mutex<Vec<Value>>,
    /// What the resolver function answers. `None` means the mount has no
    /// resolver and this is never called.
    resolution: Mutex<Option<Value>>,
    /// The resolver throws instead of answering.
    resolver_throws: bool,
}

impl ConflictMock {
    fn new() -> Self {
        Self {
            updates: Mutex::new(Vec::new()),
            resolution: Mutex::new(None),
            resolver_throws: false,
        }
    }
    fn with_resolution(resolution: Value) -> Self {
        Self {
            resolution: Mutex::new(Some(resolution)),
            ..Self::new()
        }
    }
    fn throwing_resolver() -> Self {
        Self {
            resolver_throws: true,
            ..Self::new()
        }
    }
    fn updates(&self) -> Vec<Value> {
        self.updates.lock().unwrap().clone()
    }
    fn update_count(&self) -> usize {
        self.updates.lock().unwrap().len()
    }
}

#[async_trait]
impl AdapterInvoker for ConflictMock {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _path: &str,
        input: Value,
    ) -> std::result::Result<Value, AdapterError> {
        let op = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let params = input.get("params").cloned().unwrap_or(Value::Null);
        match op {
            "capabilities" => Ok(json!({
                "can_read": true, "can_write": true, "can_update": true,
                "mutable_fields": ["unread"],
            })),
            "mapper_capabilities" => Ok(json!({ "to_external": true })),
            "to_external" => {
                let unread = input
                    .get("node")
                    .and_then(|n| n.get("properties"))
                    .and_then(|p| p.get("unread"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Ok(json!({ "payload": { "isRead": !unread } }))
            }
            "resolve_conflict" => {
                if self.resolver_throws {
                    return Err(AdapterError::Transient("resolver blew up".to_string()));
                }
                Ok(self
                    .resolution
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or(Value::Null))
            }
            "update" => {
                let mut updates = self.updates.lock().unwrap();
                updates.push(params.clone());
                if updates.len() == 1 {
                    // The message text matters: `AdapterError::classify` scans
                    // for the earlier codes first, so a conflict message
                    // containing "config_error" would be misclassified.
                    return Err(AdapterError::Conflict(
                        "the message changed at the provider".to_string(),
                    ));
                }
                Ok(json!({
                    "external_id": params.get("item_id").cloned(),
                    "etag": "v9",
                }))
            }
            _ => Ok(Value::Null),
        }
    }
}

/// A mount whose conflict policy is `conflict`, resolver at `resolver` (both
/// optional, exactly as they are on the node).
fn conflict_mount(conflict: Option<&str>, resolver: Option<&str>) -> MountConfig {
    let mut mount = state_only_mount();
    mount.write_config.conflict = conflict.map(str::to_string);
    mount.resolver_function = resolver.map(str::to_string);
    mount
}

/// Sync one read message in, then mark it unread locally: one pending edit.
async fn one_pending_edit(env: &Env, mat: &RocksDbMaterializer) -> String {
    let id = sync_in_mail(mat, false, "v1").await;
    set_bool_prop(env, &id, "unread", true).await;
    id
}

// ---------------------------------------------------------------------------
// 1. policy resolution
// ---------------------------------------------------------------------------

/// Absent means `remote_wins`, a bare `resolver_function` path means the
/// resolver, and everything unclear is REFUSED rather than defaulted.
///
/// The refusals are the point. A typo resolving to `remote_wins` would silently
/// discard the edits of a mount whose operator had explicitly configured them to
/// be kept — and `remote_wins` reports nothing, so the operator would find out
/// from a user, months later.
#[test]
fn an_unresolvable_conflict_policy_is_refused_never_defaulted() {
    let policy = |c: Option<&str>, r: Option<&str>| {
        sync::write::resolve_conflict_policy(&conflict_mount(c, r))
    };

    assert_eq!(policy(None, None).unwrap(), ConflictPolicy::RemoteWins);
    assert_eq!(
        policy(Some("remote_wins"), None).unwrap(),
        ConflictPolicy::RemoteWins
    );
    assert_eq!(
        policy(Some("LOCAL_WINS"), None).unwrap(),
        ConflictPolicy::LocalWins,
        "the value is matched case-insensitively, like every other policy"
    );
    assert_eq!(policy(Some("error"), None).unwrap(), ConflictPolicy::Error);

    // A resolver path with no `conflict` value is unambiguous: there is no other
    // reason to point a mount at one.
    assert_eq!(
        policy(None, Some("/resolvers/mail")).unwrap(),
        ConflictPolicy::Resolver("/resolvers/mail".to_string())
    );
    assert_eq!(
        policy(Some("resolver_function"), Some("/resolvers/mail")).unwrap(),
        ConflictPolicy::Resolver("/resolvers/mail".to_string())
    );

    // `resolver_function` with nothing to call is a refusal, not a fallback: the
    // operator has a merge rule in mind and running without it discards exactly
    // the edits the rule exists to keep.
    let e = policy(Some("resolver_function"), None).unwrap_err();
    assert!(e.contains("resolver_function"), "{e}");

    let typo = policy(Some("remote-wins"), None).unwrap_err();
    assert!(typo.contains("not recognized"), "{typo}");
}

/// A mount whose policy cannot be resolved does not drain AT ALL, and says so.
///
/// Checked before the first provider call rather than per item: a drain that
/// discovered this halfway would already have pushed under a policy nobody
/// chose, and those writes cannot be taken back.
#[tokio::test(flavor = "multi_thread")]
async fn a_misconfigured_policy_stops_the_drain_before_any_provider_call() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = ConflictMock::new();
    one_pending_edit(&env, &mat).await;

    let mount = conflict_mount(Some("no_such_policy"), None);
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0, "not one call may go out");
    assert_eq!(state.writeback_status.as_deref(), Some("misconfigured"));
    assert!(state
        .writeback_last_error
        .unwrap()
        .contains("not recognized"));
}

// ---------------------------------------------------------------------------
// 2. remote_wins — the default
// ---------------------------------------------------------------------------

/// The default abandons the edit: one call, nothing stamped, and the loss is
/// COUNTED rather than folded into `failed` or `skipped`.
///
/// It has to be counted somewhere. This is a local edit being thrown away, and
/// a mount doing it every run is telling its operator to pick a different
/// policy — which they cannot do if the drain reports the same clean receipt as
/// one that had nothing to do.
#[tokio::test(flavor = "multi_thread")]
async fn remote_wins_abandons_the_edit_and_says_it_did() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = ConflictMock::new();
    let id = one_pending_edit(&env, &mat).await;

    let mount = conflict_mount(None, None);
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.conflicted, 1);
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.failed, 0, "the provider answered; nothing is broken");
    assert_eq!(stats.parked, 0);
    assert_eq!(mock.update_count(), 1, "abandon means abandon, not retry");

    // The baseline must NOT move. Recording the local value as pushed would make
    // the node converge against a value the provider never accepted.
    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": false }))
    );
    assert_eq!(
        state.writeback_status, None,
        "an abandoned conflict is self-clearing; badging the mount would leave \
         every remote_wins mount permanently red"
    );
    assert!(state.writeback_last_error.unwrap().contains("discarded"));
    assert_eq!(state.last_drain.unwrap().conflicts, 1);
}

// ---------------------------------------------------------------------------
// 3. local_wins
// ---------------------------------------------------------------------------

/// `local_wins` re-sends the SAME payload with no concurrency base, and the
/// absent base is the whole mechanism: replaying the stale etag would be refused
/// identically, forever.
#[tokio::test(flavor = "multi_thread")]
async fn local_wins_retries_unconditionally_and_lands() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = ConflictMock::new();
    let id = one_pending_edit(&env, &mat).await;

    let mount = conflict_mount(Some("local_wins"), None);
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.conflicted, 0);
    assert_eq!(stats.parked, 0);

    let updates = mock.updates();
    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[0].get("etag").and_then(|v| v.as_str()),
        Some("v1"),
        "the first attempt is an ordinary optimistic write"
    );
    assert_eq!(
        updates[1].get("etag"),
        Some(&Value::Null),
        "the retry must carry NO base, or it is refused identically"
    );
    assert_eq!(updates[1].get("payload"), updates[0].get("payload"));

    // Stamped, so the drain converges rather than re-pushing every run.
    let node = node_by_id(&env, &id).await;
    assert_eq!(maybe_pushed_state(&node), Some(json!({ "unread": true })));
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v9"));
}

// ---------------------------------------------------------------------------
// 4. error — park it for a human
// ---------------------------------------------------------------------------

/// `error` parks: nothing is retried, nothing is stamped, and the mount carries
/// a `conflict` badge until a drain parks nothing.
#[tokio::test(flavor = "multi_thread")]
async fn the_error_policy_parks_the_edit_and_badges_the_mount() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = ConflictMock::new();
    let id = one_pending_edit(&env, &mat).await;

    let mount = conflict_mount(Some("error"), None);
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.parked, 1);
    assert_eq!(stats.pushed, 0);
    assert_eq!(mock.update_count(), 1);
    assert_eq!(state.writeback_status.as_deref(), Some("conflict"));
    assert!(state.writeback_last_error.unwrap().contains("'error'"));
    assert_eq!(state.last_drain.unwrap().parked, 1);

    // Pending, not consumed: the edit is still there for whoever resolves it.
    assert_eq!(
        maybe_pushed_state(&node_by_id(&env, &id).await),
        Some(json!({ "unread": false }))
    );
}

// ---------------------------------------------------------------------------
// 5. the resolver function
// ---------------------------------------------------------------------------

/// A resolver answering `merged` lands the merged values BOTH at the provider
/// and on the node.
///
/// The local landing is what separates a merge from a force. Push the merged
/// value without writing it locally and the provider holds one value, the node
/// holds another, and `__pushed_state` claims they agree — a permanent silent
/// divergence created by the mechanism that exists to end one. The second drain
/// asserts the other half: having landed, it converges and costs nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_resolver_returning_merged_lands_the_merged_fields() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    // The user marked it unread; the resolver decides the merged answer is
    // "read" — a value neither side held before, which is what makes this a
    // merge rather than a disguised local_wins or remote_wins.
    let mock = ConflictMock::with_resolution(json!({
        "resolution": "merged",
        "fields": { "unread": false },
    }));
    let id = one_pending_edit(&env, &mat).await;

    let mount = conflict_mount(None, Some("/resolvers/mail"));
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.parked, 0);
    assert_eq!(stats.conflicted, 0);

    let updates = mock.updates();
    assert_eq!(updates.len(), 2);
    assert_eq!(
        updates[1].get("payload"),
        Some(&json!({ "isRead": true })),
        "the merged value goes out through the MAPPER, not a second translation \
         in the engine"
    );
    assert_eq!(updates[1].get("etag"), Some(&Value::Null));

    // Landed locally, and recorded as the baseline.
    let node = node_by_id(&env, &id).await;
    assert_eq!(
        serde_json::to_value(node.properties.get("unread").unwrap()).unwrap(),
        json!(false),
        "the node must hold what the provider was told, or the two diverge silently"
    );
    assert_eq!(maybe_pushed_state(&node), Some(json!({ "unread": false })));
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v9"));
    assert_eq!(state.writeback_status, None);

    // And it converges: a merge that has landed costs nothing on the next run.
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 2, "the second drain is a no-op");
}

/// `local_wins` / `remote_wins` / `park` from a resolver mean exactly what the
/// fixed policies mean, so one function can decide per node.
#[tokio::test(flavor = "multi_thread")]
async fn a_resolver_can_answer_with_any_of_the_fixed_policies() {
    for (answer, pushed, conflicted, parked) in [
        (json!({ "resolution": "local_wins" }), 1, 0, 0),
        (json!({ "resolution": "remote_wins" }), 0, 1, 0),
        (
            json!({ "resolution": "park", "reason": "ask Dana" }),
            0,
            0,
            1,
        ),
    ] {
        let env = setup().await;
        let mat = RocksDbMaterializer::new(env.storage.clone());
        let mock = ConflictMock::with_resolution(answer.clone());
        one_pending_edit(&env, &mat).await;

        let mount = conflict_mount(None, Some("/resolvers/mail"));
        let c = ctx(&env, &mount, &mock, &mat);
        let mut state = MountState::default();
        let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
        let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

        assert_eq!(stats.pushed, pushed, "{answer}");
        assert_eq!(stats.conflicted, conflicted, "{answer}");
        assert_eq!(stats.parked, parked, "{answer}");
    }
}

/// Everything unclear PARKS: a resolver that throws, and one that answers
/// something this engine does not understand.
///
/// A throw classifies as `Transient` at the adapter boundary, and a transient
/// failure of the thing that decides what happens to a user's edit is not a
/// licence to pick an answer. An unrecognized `resolution` is the same
/// situation — most likely a resolver newer than the engine running it — and
/// parking is the only response that cannot be wrong: the edit survives and the
/// mismatch is visible.
#[tokio::test(flavor = "multi_thread")]
async fn a_resolver_that_throws_or_answers_nonsense_parks() {
    for mock in [
        ConflictMock::throwing_resolver(),
        ConflictMock::with_resolution(json!({ "resolution": "split_the_difference" })),
        // `merged` with nothing this mount may push is a resolver bug, not an
        // instruction — guessing which of abandon/force it meant is how an edit
        // is lost quietly.
        ConflictMock::with_resolution(json!({
            "resolution": "merged",
            "fields": { "subject": "nope" },
        })),
        ConflictMock::with_resolution(Value::Null),
    ] {
        let env = setup().await;
        let mat = RocksDbMaterializer::new(env.storage.clone());
        let id = one_pending_edit(&env, &mat).await;

        let mount = conflict_mount(None, Some("/resolvers/mail"));
        let c = ctx(&env, &mount, &mock, &mat);
        let mut state = MountState::default();
        let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
        let stats = sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

        assert_eq!(stats.parked, 1);
        assert_eq!(stats.pushed, 0);
        assert_eq!(
            mock.update_count(),
            1,
            "a parked conflict sends nothing further"
        );
        assert_eq!(state.writeback_status.as_deref(), Some("conflict"));
        assert_eq!(
            maybe_pushed_state(&node_by_id(&env, &id).await),
            Some(json!({ "unread": false })),
            "the edit stays pending for whoever resolves it"
        );
    }
}

/// A resolver sees the local node, the base etag and a two-sided field diff.
///
/// Both sides of the diff, always — including the nulls. "Did the user change
/// this, or has it simply never been pushed?" is a resolver's first question,
/// and omitting the absent half makes those two indistinguishable.
#[tokio::test(flavor = "multi_thread")]
async fn the_resolver_receives_the_diff_it_needs_to_decide() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let seen = std::sync::Arc::new(Mutex::new(Vec::<Value>::new()));
    let mock = RecordingResolver {
        inner: ConflictMock::with_resolution(json!({ "resolution": "remote_wins" })),
        seen: seen.clone(),
    };
    one_pending_edit(&env, &mat).await;

    let mount = conflict_mount(None, Some("/resolvers/mail"));
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    sync::write::drain(&c, &mut state, &mut batcher, &state_only_mode()).await;

    let input = seen.lock().unwrap()[0].clone();
    assert_eq!(input.get("base_etag").and_then(|v| v.as_str()), Some("v1"));
    assert_eq!(
        input.get("field_diff"),
        Some(&json!({ "unread": { "local": true, "pushed": false } }))
    );
    assert!(input
        .get("local")
        .and_then(|v| v.get("properties"))
        .is_some());
    assert!(
        input.get("mount").is_some(),
        "the resolver is invoked through the mapper's own plumbing, so it gets \
         the same mount snapshot"
    );
    assert!(
        input
            .get("conflict")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.contains("changed at the provider")),
        "the provider's own words are the only account of the remote side the \
         engine has: there is no adapter `get`"
    );
}

/// [`ConflictMock`] that keeps every `resolve_conflict` input.
struct RecordingResolver {
    inner: ConflictMock,
    seen: std::sync::Arc<Mutex<Vec<Value>>>,
}

#[async_trait]
impl AdapterInvoker for RecordingResolver {
    async fn invoke(
        &self,
        scope: &MountScope,
        path: &str,
        input: Value,
    ) -> std::result::Result<Value, AdapterError> {
        if input.get("operation").and_then(|v| v.as_str()) == Some("resolve_conflict") {
            self.seen.lock().unwrap().push(input.clone());
        }
        self.inner.invoke(scope, path, input).await
    }
}
