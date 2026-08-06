//! The `submit` outbox: at-most-once, proven at the one place it matters —
//! what happens when the answer to a send is not an answer.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it reuses that file's environment, mocks and helpers.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use serde_json::{json, Value};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{MountConfig, MountState, SyncConfig, WriteConfig};
use sync::materializer::MountScope;
use sync::{AdapterError, AdapterInvoker, Capabilities};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// What the mock's next `submit` does.
enum Answer {
    /// Accepted, with the provider's id.
    Ok(&'static str),
    /// The call REACHED the provider and then the adapter died — the exact
    /// shape a panic, a socket reset or a timeout takes by the time it crosses
    /// the invoker boundary. The mock records the call before failing, which is
    /// what makes "did it send?" genuinely unanswerable.
    DiedAfterSending,
    Err(AdapterError),
}

/// One invoker playing both the adapter and the bidirectional mapper, as
/// `StateOnlyMock` does for the update path.
struct SubmitMock {
    answers: Mutex<VecDeque<Answer>>,
    /// `params` of every `submit` that REACHED the adapter, in order. The
    /// at-most-once claim is a claim about the length of this list.
    submits: Mutex<Vec<Value>>,
}

impl SubmitMock {
    fn new(answers: Vec<Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            submits: Mutex::new(Vec::new()),
        })
    }
    fn submit_count(&self) -> usize {
        self.submits.lock().unwrap().len()
    }
    fn last_submit(&self) -> Value {
        self.submits.lock().unwrap().last().cloned().unwrap()
    }
}

#[async_trait]
impl AdapterInvoker for SubmitMock {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _path: &str,
        input: Value,
    ) -> Result<Value, AdapterError> {
        let op = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match op {
            "capabilities" => Ok(json!({
                "can_read": true,
                "can_write": true,
                "can_submit": true,
                "supports_idempotency_key": false,
            })),
            "mapper_capabilities" => Ok(json!({ "to_external": true })),
            "to_external" => {
                let props = input
                    .get("node")
                    .and_then(|n| n.get("properties"))
                    .cloned()
                    .unwrap_or(Value::Null);
                Ok(json!({
                    "payload": {
                        "action": props.get("action").cloned(),
                        "subject": props.get("subject").cloned(),
                    },
                }))
            }
            "submit" => {
                // Recorded BEFORE the answer is decided, deliberately: a call
                // that reached the provider and then failed is still a call that
                // reached the provider, and a mock that only counted successes
                // could not express the case this whole stage exists for.
                self.submits
                    .lock()
                    .unwrap()
                    .push(input.get("params").cloned().unwrap_or(Value::Null));
                match self.answers.lock().unwrap().pop_front() {
                    Some(Answer::Ok(id)) => Ok(json!({ "external_id": id })),
                    Some(Answer::DiedAfterSending) => Err(AdapterError::Transient(
                        "adapter panicked after the provider call".to_string(),
                    )),
                    Some(Answer::Err(e)) => Err(e),
                    None => Ok(json!({ "external_id": "AUTO" })),
                }
            }
            _ => Ok(Value::Null),
        }
    }
}

fn submit_mount() -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/outbox".to_string());
    mount.write_config = WriteConfig {
        mode: "submit".to_string(),
        ..Default::default()
    };
    mount
}

fn submit_mode() -> sync::write::WriteMode {
    sync::write::WriteMode::Submit(sync::write::SubmitPlan {
        supports_idempotency_key: false,
    })
}

/// Create one command node under the mount path, at `status`.
async fn command(env: &Env, name: &str, status: &str) -> String {
    node_under_mount(env, name, status, "raisin:OutboundMail").await
}

/// The same node at an arbitrary node type — user content that happens to live
/// under the outbox path.
async fn node_under_mount(env: &Env, name: &str, status: &str, node_type: &str) -> String {
    let tx = begin(env).await;
    let id = nanoid::nanoid!();
    let mut properties = std::collections::HashMap::new();
    properties.insert(
        "status".to_string(),
        PropertyValue::String(status.to_string()),
    );
    properties.insert("action".to_string(), PropertyValue::String("send".into()));
    properties.insert(
        "subject".to_string(),
        PropertyValue::String(format!("hello {name}")),
    );
    let node = Node {
        id: id.clone(),
        node_type: node_type.to_string(),
        name: name.to_string(),
        path: format!("{MOUNT_PATH}/{name}"),
        workspace: Some(TARGET_WS.to_string()),
        properties,
        ..Default::default()
    };
    tx.upsert_deep_node(TARGET_WS, &node, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

async fn node_of(env: &Env, id: &str) -> Node {
    let tx = begin(env).await;
    tx.get_node(TARGET_WS, id).await.unwrap().unwrap()
}

async fn status_of(env: &Env, id: &str) -> String {
    str_prop(&node_of(env, id).await, "status").unwrap_or_default()
}

/// Run one outbox drain end to end.
async fn drain(
    env: &Env,
    mount: &MountConfig,
    mock: &dyn AdapterInvoker,
) -> sync::write::DrainStats {
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(env, mount, mock, &mat);
    let mut state = MountState::default();
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    sync::write::drain(&c, &mut state, &mut batcher, &submit_mode()).await
}

// ---------------------------------------------------------------------------
// the happy path, and the two things it must leave behind
// ---------------------------------------------------------------------------

/// A queued command is issued exactly once and lands at `sent`.
///
/// The two stamps asserted here are not decoration. `attempt_id` is what makes
/// a later `unknown` answerable at the provider, and `__external_id` +
/// `__synced_at` are what hand a completed command to the mount's EXISTING
/// `ephemeral` + `ttl_seconds` cleanup — the whole garbage-collection story for
/// an outbox, and the reason there is no second reaper.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_command_is_issued_once_and_lands_at_sent() {
    let env = setup().await;
    let mount = submit_mount();
    let mock = SubmitMock::new(vec![Answer::Ok("SENT-1")]);
    let id = command(&env, "m1", "queued").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.submitted, 1);
    assert_eq!(mock.submit_count(), 1);

    let node = node_of(&env, &id).await;
    assert_eq!(str_prop(&node, "status").as_deref(), Some("sent"));
    assert_eq!(
        str_prop(&node, "sent_external_id").as_deref(),
        Some("SENT-1")
    );
    // Presence, not the stored representation: an RFC 3339 string round-trips
    // through the untagged `PropertyValue` as a `Date`, which is fine — what
    // matters is that a completed command records WHEN.
    assert!(node.properties.contains_key("sent_at"));
    assert!(
        str_prop(&node, "attempt_id").is_some(),
        "the attempt id must survive on the node: it is what makes an ambiguous \
         outcome answerable at the provider"
    );
    assert_eq!(str_prop(&node, "__external_id").as_deref(), Some("SENT-1"));
    assert!(
        node.properties.contains_key("__synced_at")
            && str_prop(&node, "__mount_id").as_deref() == Some(MOUNT_ID),
        "a completed command must be collectable by the mount's own TTL cleanup, \
         which reads `__synced_at` off a mount-owned node"
    );

    // The idempotency key carries the attempt, so a provider that CAN honour
    // one deduplicates per attempt rather than per command.
    let key = mock.last_submit()["idempotency_key"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(key.contains(&id), "{key}");
    assert!(
        key.contains(str_prop(&node, "attempt_id").unwrap().as_str()),
        "{key}"
    );

    // Idempotent: a second drain finds nothing to do, because `sent` is not a
    // status this path acts on.
    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.submitted, 0);
    assert_eq!(mock.submit_count(), 1);
}

/// A DRAFT is never sent. Composing is not authorizing.
///
/// The failure this prevents is an agent or a half-built compose UI creating a
/// command node without thinking about `status` and thereby mailing someone.
#[tokio::test(flavor = "multi_thread")]
async fn a_draft_is_never_issued() {
    let env = setup().await;
    let mount = submit_mount();
    let mock = SubmitMock::new(vec![Answer::Ok("NOPE")]);
    let id = command(&env, "d1", "draft").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.submitted, 0);
    assert_eq!(mock.submit_count(), 0, "no draft may reach the adapter");
    assert_eq!(status_of(&env, &id).await, "draft");
}

// ---------------------------------------------------------------------------
// THE test: an ambiguous failure is never retried
// ---------------------------------------------------------------------------

/// An adapter that dies AFTER the provider call leaves the command at
/// `unknown`, and no subsequent drain ever sends it again.
///
/// This is the whole stage in one test. Every other path in this engine treats
/// an unrecognized failure as transient and retries it; here that default would
/// mean a second email. The command is parked, the reason is recorded on the
/// node, and only a person can move it back to `queued`.
#[tokio::test(flavor = "multi_thread")]
async fn a_death_after_the_provider_call_parks_at_unknown_and_is_never_retried() {
    let env = setup().await;
    let mount = submit_mount();
    // Note the SECOND answer: if the drain ever retried, it would succeed and
    // the assertions below would be about a `sent` node. It exists so the test
    // fails loudly on a retry rather than quietly on a second failure.
    let mock = SubmitMock::new(vec![Answer::DiedAfterSending, Answer::Ok("SECOND-SEND")]);
    let id = command(&env, "m1", "queued").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.unresolved, 1, "an ambiguous outcome, not a failure");
    assert_eq!(stats.submitted, 0);
    assert_eq!(mock.submit_count(), 1);
    assert_eq!(status_of(&env, &id).await, "unknown");
    let reason = str_prop(&node_of(&env, &id).await, "last_error").unwrap();
    assert!(
        reason.contains("NOT retried"),
        "the node must say why it is stuck and that nothing will move it: {reason}"
    );

    // Two more drains. Nothing reaches the adapter, ever.
    for _ in 0..2 {
        let stats = drain(&env, &mount, mock.as_ref()).await;
        assert_eq!(stats.submitted, 0);
        assert_eq!(
            stats.unresolved, 0,
            "an already-parked command is not re-parked"
        );
    }
    assert_eq!(
        mock.submit_count(),
        1,
        "a retried send is a duplicate email; exactly one submit may ever reach \
         the adapter for one command"
    );
    assert_eq!(status_of(&env, &id).await, "unknown");
}

/// `rate_limited` — and ONLY `rate_limited` — comes back.
///
/// It is the one answer that proves the provider did not act, which is what
/// makes resending it safe. The contrast with the test above is the whole
/// classifier.
#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_command_returns_to_queued_and_is_issued_again() {
    let env = setup().await;
    let mount = submit_mount();
    let mock = SubmitMock::new(vec![
        Answer::Err(AdapterError::RateLimited),
        Answer::Ok("SENT-2"),
    ]);
    let id = command(&env, "m1", "queued").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.requeued, 1);
    assert_eq!(stats.submitted, 0);
    assert_eq!(
        status_of(&env, &id).await,
        "queued",
        "the command must go BACK to queued, not park"
    );

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.submitted, 1);
    assert_eq!(mock.submit_count(), 2);
    assert_eq!(status_of(&env, &id).await, "sent");
}

/// A definitive pre-effect rejection is terminal `failed`, not `unknown`.
///
/// The difference is what a person may safely do next: a `failed` command can
/// be requeued freely because nothing was sent; an `unknown` one must be checked
/// at the provider first.
#[tokio::test(flavor = "multi_thread")]
async fn a_definitive_rejection_fails_rather_than_parking_as_unknown() {
    let env = setup().await;
    let mount = submit_mount();
    let mock = SubmitMock::new(vec![Answer::Err(AdapterError::Config(
        "config_error: this mount cannot send".to_string(),
    ))]);
    let id = command(&env, "m1", "queued").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.abandoned, 1);
    assert_eq!(stats.unresolved, 0);
    assert_eq!(status_of(&env, &id).await, "failed");
    assert_eq!(mock.submit_count(), 1);
}

/// A command left `sending` by a run that died moves to `unknown` — NOT back to
/// `queued` — and never reaches the adapter.
///
/// This is what the durable claim buys. Without it the crashed command still
/// reads `queued` and every subsequent drain resends it; with it the ambiguity
/// is bounded to the single recorded attempt.
#[tokio::test(flavor = "multi_thread")]
async fn a_command_left_claimed_by_a_dead_run_parks_instead_of_resending() {
    let env = setup().await;
    let mount = submit_mount();
    let mock = SubmitMock::new(vec![Answer::Ok("WOULD-BE-A-DUPLICATE")]);
    let id = command(&env, "m1", "sending").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;
    assert_eq!(stats.unresolved, 1);
    assert_eq!(
        mock.submit_count(),
        0,
        "a claim that outlived its run must never be re-issued"
    );
    assert_eq!(status_of(&env, &id).await, "unknown");
}

// ---------------------------------------------------------------------------
// provenance: the outbox only owns its own commands
// ---------------------------------------------------------------------------

/// A node under the outbox path that is not one of this mount's command types is
/// never claimed, never mutated and never sent — whatever its `status` says.
///
/// The outbox cannot use the ownership check every other write path uses:
/// a command is authored by a user and carries no `__mount_id` until the drain
/// stamps one on it. So the node's TYPE is the provenance rule, and without it
/// the drain acts on "any node under this path whose `status` reads `queued`" —
/// a task, a draft order, an agent's scratch record. Claiming one writes engine
/// properties onto a stranger's node; with a permissive `to_external` it emails
/// somebody.
///
/// The `sending` half is the worse one: that arm consults no mapper at all, so a
/// user's node is rewritten to `unknown` with an engine `last_error` purely
/// because of a coincidental property value.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_of_another_type_under_the_outbox_is_never_claimed_or_sent() {
    let env = setup().await;
    let mount = submit_mount();
    let mock = SubmitMock::new(vec![Answer::Ok("SHOULD-NEVER-BE-SENT")]);
    let task = node_under_mount(&env, "t1", "queued", "raisin:Node").await;
    let claimed = node_under_mount(&env, "t2", "sending", "raisin:Node").await;

    let stats = drain(&env, &mount, mock.as_ref()).await;

    assert_eq!(mock.submit_count(), 0, "user content must never be sent");
    assert_eq!(stats.submitted, 0);
    assert_eq!(
        stats.unresolved, 0,
        "a foreign node must not be parked as though it were this mount's command"
    );

    for id in [&task, &claimed] {
        let node = node_of(&env, id).await;
        assert!(
            !node.properties.contains_key("attempt_id")
                && !node.properties.contains_key("__write_seq")
                && !node.properties.contains_key("last_error"),
            "the write engine stamped a node it does not own: {:?}",
            node.properties
        );
    }
    assert_eq!(status_of(&env, &task).await, "queued");
    assert_eq!(status_of(&env, &claimed).await, "sending");

    // A mount whose outbox holds its own type says so, and then it sends.
    let mut declared = submit_mount();
    declared.write_config.command_node_types = vec!["raisin:Node".to_string()];
    let stats = drain(&env, &declared, mock.as_ref()).await;
    assert_eq!(stats.submitted, 1, "an opted-in type is a command");
    assert_eq!(status_of(&env, &task).await, "sent");
}

// ---------------------------------------------------------------------------
// resolution: a mount is never more able to send than its adapter says
// ---------------------------------------------------------------------------

/// `submit` resolves honestly against the adapter and the mapper.
///
/// Before this stage the mode was REFUSED as "not implemented yet"; the risk on
/// the way in is the opposite one — a mount resolving to `Submit` against an
/// adapter with no `submit` operation behind it, which fails at drain time
/// after the command has already been claimed.
#[test]
fn submit_is_refused_unless_both_the_adapter_and_the_mapper_can_send() {
    use sync::write::{resolve_mode, WriteMode};
    let wc = WriteConfig {
        mode: "submit".to_string(),
        ..Default::default()
    };
    let full = Capabilities {
        can_write: true,
        can_submit: true,
        ..Default::default()
    };

    assert!(matches!(
        resolve_mode(&wc, &full, &MapperWriteback::Supported),
        WriteMode::Submit(_)
    ));

    // An adapter that only READS. `can_update` is irrelevant either way — a
    // command patches nothing — so requiring it here would refuse a good outbox
    // for lacking an operation it never calls.
    match resolve_mode(&wc, &Capabilities::fallback(), &MapperWriteback::Supported) {
        WriteMode::Refused(r) => {
            assert!(r.contains("can_write") && r.contains("can_submit"), "{r}")
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // A write-capable adapter behind a mapper that cannot translate outward is
    // NOT a sending mount. Issuing a guess here means emailing someone.
    match resolve_mode(&wc, &full, &MapperWriteback::NotImplemented) {
        WriteMode::Refused(r) => assert!(r.contains("to_external"), "{r}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}
