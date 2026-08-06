//! Attachments as `raisin:Asset` SUBNODES: staging, the reconcile hazard, and
//! the delete cascade.
//!
//! Every failure here is silent by construction — no error, no failed count, a
//! run that reports success — so each test asserts the observable consequence
//! (a node that is there, a node that is gone) rather than an internal flag.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{child_external_id, MountState, SyncConfig};

/// One provider message and the attachments currently hanging off it.
#[derive(Clone)]
struct Message {
    id: &'static str,
    etag: String,
    /// `(provider attachment id, filename)`.
    attachments: Vec<(String, String)>,
}

/// An adapter + mapper in one, because [`ctx`] wires a single invoker for both.
///
/// `list` returns the messages; `to_node` maps one into a `raisin:Mail` with a
/// `children` array of `raisin:Asset` entries — the shape a real mail mapper
/// emits once it reads `metadata.attachments`.
struct MailWithAttachments {
    messages: Mutex<Vec<Message>>,
    /// Every `to_node` the mapper was actually asked for. The etag skip-write is
    /// supposed to keep this empty on an unchanged re-sync.
    mapped: Mutex<Vec<String>>,
    /// Queued `get_changes` pages, oldest first.
    changes: Mutex<Vec<Value>>,
}

impl MailWithAttachments {
    fn new(messages: Vec<Message>) -> Self {
        Self {
            messages: Mutex::new(messages),
            mapped: Mutex::new(Vec::new()),
            changes: Mutex::new(Vec::new()),
        }
    }

    fn set(&self, messages: Vec<Message>) {
        *self.messages.lock().unwrap() = messages;
    }

    fn mapped_count(&self) -> usize {
        self.mapped.lock().unwrap().len()
    }

    /// Queue a `get_changes` page reporting one message as deleted upstream.
    fn push_deletion(&self, id: &str) {
        self.changes.lock().unwrap().push(json!({
            "items": [{
                "type": "deleted",
                "item": { "external_id": id, "name": format!("{id}.eml"), "is_folder": false },
                "relative_path": format!("{id}.eml"),
            }],
            "next_token": "t1",
        }));
    }
}

#[async_trait]
impl AdapterInvoker for MailWithAttachments {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _adapter_path: &str,
        input: Value,
    ) -> std::result::Result<Value, AdapterError> {
        match input.get("operation").and_then(|v| v.as_str()) {
            Some("list") => {
                let items: Vec<Value> = self
                    .messages
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|m| {
                        json!({
                            "external_id": m.id,
                            "name": format!("{}.eml", m.id),
                            "is_folder": false,
                            "etag": m.etag,
                            "mime_type": "message/rfc822",
                        })
                    })
                    .collect();
                Ok(json!({ "items": items, "next_cursor": null }))
            }
            Some("to_node") | None => {
                let item = input.get("external_item").cloned().unwrap_or(Value::Null);
                let id = item
                    .get("external_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.mapped.lock().unwrap().push(id.clone());
                let msg = self
                    .messages
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|m| m.id == id)
                    .cloned();
                let Some(msg) = msg else {
                    return Ok(Value::Null);
                };
                let children: Vec<Value> = msg
                    .attachments
                    .iter()
                    .map(|(aid, name)| {
                        json!({
                            "name": name,
                            "node_type": "raisin:Asset",
                            "external_id": aid,
                            "properties": {
                                "title": name,
                                "file_type": "application/pdf",
                                "file_size": 1234,
                                "inline": false,
                            },
                        })
                    })
                    .collect();
                Ok(json!({
                    "node_type": "raisin:Mail",
                    "name": format!("{}.eml", msg.id),
                    "properties": { "subject": "hello", "has_attachments": !children.is_empty() },
                    "children": children,
                }))
            }
            Some("get_changes") => {
                let mut queued = self.changes.lock().unwrap();
                if queued.is_empty() {
                    Ok(json!({ "items": [], "next_token": null }))
                } else {
                    Ok(queued.remove(0))
                }
            }
            _ => Ok(Value::Null),
        }
    }
}

fn msg(id: &'static str, etag: &str, attachments: &[(&str, &str)]) -> Message {
    Message {
        id,
        etag: etag.to_string(),
        attachments: attachments
            .iter()
            .map(|(a, n)| (a.to_string(), n.to_string()))
            .collect(),
    }
}

fn mail_mount() -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/mail".to_string());
    mount
}

async fn run_full(env: &Env, mount: &MountConfig, mock: &MailWithAttachments) {
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(env, mount, mock, &mat);
    let mut state = MountState::default();
    sync::full::run(&c, &mut state).await.unwrap();
}

async fn node_at(env: &Env, path: &str) -> Option<Node> {
    all_nodes(env, TARGET_WS)
        .await
        .into_iter()
        .find(|n| n.path == path)
}

/// The shape the whole stage exists to produce: one node per attachment, under
/// the message, as a `raisin:Asset` — not an `attachments` array property.
#[tokio::test(flavor = "multi_thread")]
async fn a_mapper_child_becomes_an_asset_subnode() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![msg("M1", "e1", &[("att-7", "quote.pdf")])]);

    run_full(&env, &mount, &mock).await;

    let mail = node_at(&env, "/drive/M1.eml").await.expect("mail node");
    assert_eq!(mail.node_type, "raisin:Mail");

    let att = node_at(&env, "/drive/M1.eml/quote.pdf")
        .await
        .expect("attachment materialized as a child of the mail");
    assert_eq!(att.node_type, "raisin:Asset");
    // Metadata only. `file` absent IS the contract: it means "not fetched yet",
    // and is what the on-demand `get_content` fetch keys off.
    assert!(
        !att.properties.contains_key("file"),
        "sync must write metadata only; bytes come from get_content on demand"
    );
    assert_eq!(
        str_prop(&att, "file_type").as_deref(),
        Some("application/pdf")
    );

    // NAMESPACED. A Graph attachment id is unique only within its message, so a
    // bare "att-7" on a second message would match this node by __external_id
    // and the sync would relocate one node back and forth forever.
    assert_eq!(
        str_prop(&att, "__external_id"),
        Some(child_external_id("M1", "att-7"))
    );
    assert_eq!(str_prop(&att, "__mount_id").as_deref(), Some(MOUNT_ID));
}

/// Two messages with the SAME provider attachment id must produce two nodes.
///
/// Without the namespace both attachments carry `__external_id = "1"`, the sync
/// index matches the second onto the first's node, and one of the two mails
/// silently loses its attachment while the other's moves between them on
/// alternate runs.
#[tokio::test(flavor = "multi_thread")]
async fn the_same_attachment_id_on_two_messages_makes_two_nodes() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![
        msg("M1", "e1", &[("1", "a.pdf")]),
        msg("M2", "e1", &[("1", "b.pdf")]),
    ]);

    run_full(&env, &mount, &mock).await;

    assert!(node_at(&env, "/drive/M1.eml/a.pdf").await.is_some());
    assert!(node_at(&env, "/drive/M2.eml/b.pdf").await.is_some());
}

/// A message deleted through the DELTA feed must take its attachments with it.
///
/// The delta path is where this is load-bearing: it names ONE external id and
/// deletes exactly that node. `TransactionalContext::delete_node` explicitly
/// does not descend (its own doc comment says so), so without the materializer
/// cascading, the asset children survive as mount-owned orphans under a
/// tombstoned parent path — invisible in the tree, still matching `__mount_id`
/// queries, and never revisited, because the provider will never mention that
/// message again.
///
/// (A full walk happens to clean them up for a different reason — reconcile
/// deletes everything it did not see — which is exactly why the delta path
/// needs its own test rather than inheriting confidence from that one.)
#[tokio::test(flavor = "multi_thread")]
async fn a_delta_delete_cascades_to_attachment_children() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![msg("M1", "e1", &[("att-7", "quote.pdf")])]);
    run_full(&env, &mount, &mock).await;
    assert!(node_at(&env, "/drive/M1.eml/quote.pdf").await.is_some());

    mock.push_deletion("M1");
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    sync::delta::run(&c, &mut state).await.unwrap();

    assert!(
        node_at(&env, "/drive/M1.eml").await.is_none(),
        "the message the delta named is gone"
    );
    assert!(
        node_at(&env, "/drive/M1.eml/quote.pdf").await.is_none(),
        "its attachment child must go with it, not survive as an orphan the \
         provider will never mention again"
    );
}

/// The same cascade under a full walk, where the parent and the child are BOTH
/// reconciled in one batch.
///
/// Reconcile stages a `Delete` per unseen external id and the batch's order is a
/// HashMap iteration order, so the child's own op runs before the parent's about
/// half the time. The index is only updated after the transaction commits, so
/// the cascade still sees the child as live — and propagating that `NotFound`
/// aborted the PARENT's delete. Flaky by construction: the message survived a
/// reconcile that had just deleted its attachment.
#[tokio::test(flavor = "multi_thread")]
async fn reconciling_a_message_and_its_attachment_together_removes_both() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![msg(
        "M1",
        "e1",
        &[
            ("a1", "one.pdf"),
            ("a2", "two.pdf"),
            ("a3", "three.pdf"),
            ("a4", "four.pdf"),
        ],
    )]);
    run_full(&env, &mount, &mock).await;

    // The provider no longer lists M1: a full walk reconciles the whole subtree.
    mock.set(vec![msg("M2", "e2", &[])]);
    run_full(&env, &mount, &mock).await;

    let survivors: Vec<String> = all_nodes(&env, TARGET_WS)
        .await
        .into_iter()
        .map(|n| n.path)
        .filter(|p| p.starts_with("/drive/M1.eml"))
        .collect();
    assert!(
        survivors.is_empty(),
        "nothing of the deleted message may survive: {survivors:?}"
    );
}

/// The reconcile hazard, and the sharpest one in the whole feature.
///
/// A full walk deletes every mount-owned node it did not see. Attachments are
/// mount-owned and carry an `__external_id`, but the PROVIDER never lists them —
/// they exist only inside a message's mapping. And on an unchanged re-sync the
/// etag skip-write returns before the mapper ever runs, so nothing re-derives
/// them either.
///
/// Result without the fix: the very first idle re-sync of a mailbox deletes
/// every attachment node in it, and the next changed sync recreates them. Churn
/// forever, on the cheapest path there is.
#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_message_keeps_its_attachments_across_a_re_sync() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![msg("M1", "e1", &[("att-7", "quote.pdf")])]);

    run_full(&env, &mount, &mock).await;
    assert!(node_at(&env, "/drive/M1.eml/quote.pdf").await.is_some());
    let mapped_after_first = mock.mapped_count();

    // Same etag: the second walk skips the message before mapping it.
    run_full(&env, &mount, &mock).await;

    assert_eq!(
        mock.mapped_count(),
        mapped_after_first,
        "unchanged etag must still skip the mapper — the skip path is the one under test"
    );
    assert!(
        node_at(&env, "/drive/M1.eml/quote.pdf").await.is_some(),
        "an idle re-sync must not reconcile away attachments the provider never lists"
    );
}

/// The other half of the same rule: an attachment that really is gone upstream
/// must be reconciled away. A blanket "never delete a child" would have passed
/// the test above and left deleted attachments forever.
#[tokio::test(flavor = "multi_thread")]
async fn an_attachment_removed_upstream_is_reconciled_away() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![msg(
        "M1",
        "e1",
        &[("att-7", "quote.pdf"), ("att-8", "logo.png")],
    )]);
    run_full(&env, &mount, &mock).await;
    assert!(node_at(&env, "/drive/M1.eml/logo.png").await.is_some());

    // The message changed (new etag) and now carries one attachment.
    mock.set(vec![msg("M1", "e2", &[("att-7", "quote.pdf")])]);
    run_full(&env, &mount, &mock).await;

    assert!(node_at(&env, "/drive/M1.eml/quote.pdf").await.is_some());
    assert!(
        node_at(&env, "/drive/M1.eml/logo.png").await.is_none(),
        "an attachment the mapper no longer emits is gone upstream"
    );
}

/// A provider-supplied filename is untrusted input, and a child's name becomes a
/// path SEGMENT. `../../etc/passwd` must not graft a node outside the mount.
#[tokio::test(flavor = "multi_thread")]
async fn a_traversing_attachment_name_cannot_escape_the_message() {
    let env = setup().await;
    let mount = mail_mount();
    let mock = MailWithAttachments::new(vec![msg("M1", "e1", &[("att-7", "../../evil.pdf")])]);

    run_full(&env, &mount, &mock).await;

    let paths: Vec<String> = all_nodes(&env, TARGET_WS)
        .await
        .into_iter()
        .map(|n| n.path)
        .collect();
    // The attachment landed as exactly ONE segment directly under its message:
    // the separators were neutralised, not honoured. Asserting on EVERY node
    // under the message, not the first one found — an unsanitized name also
    // makes `upsert_deep_node` auto-create `/drive/M1.eml/..` as a folder, which
    // on its own looks like a single innocent segment.
    let under_message: Vec<&String> = paths
        .iter()
        .filter(|p| p.starts_with("/drive/M1.eml/"))
        .collect();
    assert_eq!(
        under_message.len(),
        1,
        "one attachment must produce one node, got {under_message:?}"
    );
    assert!(
        !under_message[0]["/drive/M1.eml/".len()..].contains('/'),
        "a traversing name must collapse to one segment, got {}",
        under_message[0]
    );
    assert!(
        paths.iter().all(|p| !p.contains("/..")),
        "no materialized path may contain a traversal segment: {paths:?}"
    );
}
