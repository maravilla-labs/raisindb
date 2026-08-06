//! `mirror` mode: the first write path that can destroy remote data, and the
//! rails that stand in front of it.
//!
//! The unit-level rail arithmetic (a 200k-node mount with 11 deletes passing, a
//! small mount with 6 blocking, the bulk flag firing regardless of count) lives
//! in `write::guard`'s own tests, where it needs no database. What is here is
//! everything those cannot prove: that the rails are actually WIRED — that a
//! real multi-node delete transaction over a real mounted path reaches the
//! adapter zero times.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment, mocks and helpers.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{MappedNode, MountConfig, MountState, WriteConfig, BULK_REVISION_THRESHOLD};
use sync::materializer::{BatchOp, MountScope, VirtualMeta};
use sync::write::reconcile::reconcile_mount;
use sync::{AdapterError, AdapterInvoker, Capabilities, MapperWriteback, SyncCtx};

const LIMIT: usize = 500;

// ---------------------------------------------------------------------------
// a mirror-capable provider
// ---------------------------------------------------------------------------

/// One invoker playing the adapter and the mount's bidirectional mapper, as
/// [`super::StateOnlyMock`] does for `state_only` — but declaring the full
/// mirror capability set, and counting every `delete` that reaches it.
///
/// The delete counter is the assertion the headline test rests on: "the rails
/// held" is not a state flag, it is the absence of a provider call.
#[derive(Default)]
struct MirrorMock {
    deletes: Mutex<Vec<Value>>,
    updates: Mutex<Vec<Value>>,
}

impl MirrorMock {
    fn delete_count(&self) -> usize {
        self.deletes.lock().unwrap().len()
    }
    fn deletes(&self) -> Vec<Value> {
        self.deletes.lock().unwrap().clone()
    }
    fn update_count(&self) -> usize {
        self.updates.lock().unwrap().len()
    }
}

/// What [`MirrorMock`] declares, and what the tests resolve modes against.
fn mirror_caps() -> Capabilities {
    Capabilities {
        can_read: true,
        can_write: true,
        can_create: true,
        can_update: true,
        can_delete: true,
        supports_changes: true,
        supports_trash: true,
        mutable_fields: vec!["title".to_string()],
        default_delete_policy: Some("detach".to_string()),
        ..Default::default()
    }
}

#[async_trait]
impl AdapterInvoker for MirrorMock {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _path: &str,
        input: Value,
    ) -> std::result::Result<Value, AdapterError> {
        let params = input.get("params").cloned().unwrap_or(Value::Null);
        match input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("")
        {
            "capabilities" => Ok(serde_json::to_value(mirror_caps()).unwrap()),
            "mapper_capabilities" => Ok(json!({ "to_external": true })),
            "to_external" => {
                let title = input
                    .get("node")
                    .and_then(|n| n.get("properties"))
                    .and_then(|p| p.get("title"))
                    .cloned();
                match title {
                    Some(t) => Ok(json!({ "payload": { "name": t } })),
                    None => Ok(Value::Null),
                }
            }
            "update" => {
                self.updates.lock().unwrap().push(params.clone());
                Ok(json!({ "external_id": params.get("item_id").cloned(), "etag": "v9" }))
            }
            "delete" => {
                self.deletes.lock().unwrap().push(params);
                Ok(json!({ "deleted": true }))
            }
            _ => Ok(Value::Null),
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn mirror_mount(delete_policy: &str) -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/drive".to_string());
    mount.write_config = WriteConfig {
        mode: "mirror".to_string(),
        mutable_fields: vec!["title".to_string()],
        delete_policy: Some(delete_policy.to_string()),
        ..Default::default()
    };
    mount
}

fn mirror_mode(mount: &MountConfig) -> sync::write::WriteMode {
    sync::write::resolve_mode(
        &mount.write_config,
        &mirror_caps(),
        &MapperWriteback::Supported,
    )
}

/// Materialize `n` mount-owned nodes the way a sync would, returning their ids.
async fn sync_in(mat: &RocksDbMaterializer, n: usize) -> Vec<String> {
    let mut index = mat.load_index(&scope()).await.unwrap();
    let ops: Vec<BatchOp> = (0..n)
        .map(|i| {
            let mut properties = serde_json::Map::new();
            properties.insert("title".to_string(), Value::String(format!("f{i:03}.txt")));
            BatchOp::Upsert {
                rel_path: format!("f{i:03}.txt"),
                mapped: MappedNode {
                    node_type: "raisin:Node".to_string(),
                    name: Some(format!("f{i:03}")),
                    properties,
                },
                virt: VirtualMeta {
                    mount_id: MOUNT_ID.to_string(),
                    external_id: format!("F{i:03}"),
                    etag: Some("v1".to_string()),
                    synced_at: Utc::now().to_rfc3339(),
                },
            }
        })
        .collect();
    mat.apply_batch(&scope(), &mut index, ops).await.unwrap();
    let mut ids: Vec<(String, String)> = index
        .virtual_nodes()
        .into_iter()
        .map(|n| (n.external_id, n.id))
        .collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

/// Delete nodes in ONE transaction — the shape a mis-scoped
/// `DELETE FROM 'ws' WHERE path LIKE '/drive/%'` commits. Bulk SQL DML is not
/// silent: it goes through `TransactionalContext` and emits one `RevisionMeta`
/// whose `changed_nodes` holds every node it touched, which is precisely the
/// signal the bulk rail reads.
async fn bulk_delete(env: &Env, ids: &[String]) {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch("main").unwrap();
    tx.set_actor("alice").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_message("DELETE FROM 'default' WHERE path LIKE '/drive/%'")
        .unwrap();
    for id in ids {
        tx.delete_node(TARGET_WS, id).await.unwrap();
    }
    tx.commit().await.unwrap();
}

/// Walk the change feed from the bottom so every commit in the test is seen.
async fn reconcile(env: &Env, mount: &MountConfig, state: &mut MountState) {
    if state.writeback_revision.is_none() {
        state.writeback_revision = Some(raisin_hlc::HLC::new(0, 0).to_string());
    }
    reconcile_mount(&env.storage, TENANT, REPO, mount, state, LIMIT)
        .await
        .unwrap();
}

async fn drain(
    env: &Env,
    mount: &MountConfig,
    mock: &MirrorMock,
    state: &mut MountState,
) -> sync::write::DrainStats {
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(env, mount, mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    sync::write::drain(&c, state, &mut batcher, &mirror_mode(mount)).await
}

// ---------------------------------------------------------------------------
// 1. mirror is wired
// ---------------------------------------------------------------------------

/// `mirror` used to be REFUSED by name — "write mode 'mirror' is not
/// implemented yet". It now resolves, and the verdict the console reads is the
/// same decision the drain obeys rather than a second copy of it.
#[test]
fn mirror_resolves_instead_of_being_refused_by_name() {
    let mount = mirror_mount("trash");
    match mirror_mode(&mount) {
        sync::write::WriteMode::Mirror(_) => {}
        other => panic!("mirror must resolve, got {other:?}"),
    }
    assert_eq!(
        sync::write::writeback_verdict(
            &mount.write_config,
            &mirror_caps(),
            &MapperWriteback::Supported
        ),
        (Some(true), None)
    );

    // The deprecated `writeback: "write_through"` alias is the same mode, not a
    // second one — and it now resolves through the same function.
    let alias = WriteConfig {
        writeback: "write_through".to_string(),
        mutable_fields: vec!["title".to_string()],
        ..Default::default()
    };
    assert!(alias.wants_mirror());
    assert!(matches!(
        sync::write::resolve_mode(&alias, &mirror_caps(), &MapperWriteback::Supported),
        sync::write::WriteMode::Mirror(_)
    ));

    // A mirror-capable adapter is NOT thereby a sending one. `submit` is
    // refused here for the reason that is actually true — no `can_submit` —
    // rather than promoted on the strength of create/update/delete, which say
    // nothing about whether this provider can issue a command.
    let submit = WriteConfig {
        mode: "submit".to_string(),
        ..Default::default()
    };
    match sync::write::resolve_mode(&submit, &mirror_caps(), &MapperWriteback::Supported) {
        sync::write::WriteMode::Refused(r) => assert!(r.contains("can_submit"), "{r}"),
        other => panic!("expected submit to be refused, got {other:?}"),
    }
}

/// A mirror needs create AND delete, not just the update a `state_only` mount
/// needs — and the refusal names which one is missing rather than saying
/// "unsupported".
#[test]
fn a_mirror_on_a_read_only_adapter_is_refused_with_the_missing_ops() {
    let caps = Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        mutable_fields: vec!["title".to_string()],
        ..Default::default()
    };
    match sync::write::resolve_mode(
        &mirror_mount("detach").write_config,
        &caps,
        &MapperWriteback::Supported,
    ) {
        sync::write::WriteMode::Refused(r) => {
            assert!(r.contains("can_delete"), "{r}");
            // NOT `can_create`. This mount declares no `create_node_types`, so
            // the engine will never call the adapter's `create` for it, and
            // demanding the capability would refuse an adapter that mirrors
            // perfectly well for an operation it is never asked to perform.
            // Gating every mirror mount on the rarest capability is what made
            // `mirror` look unimplementable against providers that update and
            // delete fine.
            assert!(
                !r.contains("can_create"),
                "create was demanded from a mount that never creates: {r}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// ...but a mount that DOES opt into local create must have the capability.
///
/// The other half of the rule above, and the one that keeps it honest: relaxing
/// the gate must not make `create_node_types` a setting that silently does
/// nothing against an adapter that cannot create.
#[test]
fn a_mirror_that_creates_locally_still_demands_can_create() {
    let caps = Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        can_delete: true,
        mutable_fields: vec!["title".to_string()],
        ..Default::default()
    };
    let mut mount = mirror_mount("detach");
    mount.write_config.create_node_types = vec!["raisin:Event".to_string()];
    match sync::write::resolve_mode(&mount.write_config, &caps, &MapperWriteback::Supported) {
        sync::write::WriteMode::Refused(r) => assert!(r.contains("can_create"), "{r}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 2. the headline: a mis-scoped bulk delete reaches the provider zero times
// ---------------------------------------------------------------------------

/// **The reason the rails exist.** A mis-scoped bulk delete over a mounted path
/// must reach the adapter's `delete` ZERO times, park every intent, and say so.
///
/// Everything about the setup is real: real synced nodes, a real multi-node
/// delete transaction, the real `RevisionMeta` walk that recovers each node's
/// provider id from its MVCC pre-image, and the real drain. The only mock is the
/// provider, and it is mocked precisely so it can assert it was never called.
///
/// Note what is NOT lost. The deletes are still queued on
/// `writeback_pending_deletes`, so an operator who decides the delete was
/// intended can release exactly this batch — see the confirmation test below.
#[tokio::test(flavor = "multi_thread")]
async fn a_mis_scoped_bulk_delete_never_reaches_the_provider() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    // `purge` — the most destructive policy there is, chosen deliberately: the
    // rails have to hold for the configuration where holding matters.
    let mount = mirror_mount("purge");
    let mock = MirrorMock::default();

    let ids = sync_in(&mat, BULK_REVISION_THRESHOLD + 5).await;
    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await; // walk past the import

    bulk_delete(&env, &ids).await;
    reconcile(&env, &mount, &mut state).await;
    assert_eq!(
        state.writeback_pending_deletes.len(),
        ids.len(),
        "every delete must be detected before anything can refuse it"
    );

    let stats = drain(&env, &mount, &mock, &mut state).await;

    assert_eq!(
        mock.delete_count(),
        0,
        "a mis-scoped bulk delete reached the provider"
    );
    assert!(stats.blocked, "the block must be recorded on the drain");
    assert_eq!(stats.deleted, 0);
    let block = state
        .writeback_blocked
        .clone()
        .expect("writeback_blocked must be set");
    assert_eq!(block.deletes as usize, ids.len());
    assert!(
        block.reason.contains("bulk"),
        "the message must name the signal that fired, or the operator raises the \
         wrong limit: {}",
        block.reason
    );
    assert!(
        !block.token.is_empty() && block.reason.contains(&block.token),
        "the release instruction must carry the token"
    );

    // Nothing was lost, and the reason reached the operator-facing field.
    assert_eq!(state.writeback_pending_deletes.len(), ids.len());
    assert_eq!(
        state.writeback_last_error.as_deref(),
        Some(&block.reason[..])
    );
    assert!(
        state.last_drain.as_ref().unwrap().blocked,
        "a blocked drain must not be indistinguishable from an idle one"
    );

    // A second drain does not wear the block down: it blocks identically, with
    // the same token, so an operator's review is not invalidated by a tick.
    let again = drain(&env, &mount, &mock, &mut state).await;
    assert!(again.blocked);
    assert_eq!(mock.delete_count(), 0);
    assert_eq!(state.writeback_blocked.unwrap().token, block.token);
}

/// A block parks the WHOLE drain, not only its deletes (§9.4): an operator
/// reviewing an unexpected mass delete should not have this mount making other
/// outbound changes meanwhile.
#[tokio::test(flavor = "multi_thread")]
async fn a_block_parks_field_updates_too() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = mirror_mount("purge");
    let mock = MirrorMock::default();

    let ids = sync_in(&mat, BULK_REVISION_THRESHOLD + 2).await;
    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;

    // One survivor, locally renamed — an ordinary pending update.
    let survivor = ids[0].clone();
    let tx = begin(&env).await;
    let mut node = tx.get_node(TARGET_WS, &survivor).await.unwrap().unwrap();
    node.properties.insert(
        "title".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("renamed.txt".to_string()),
    );
    tx.upsert_node(TARGET_WS, &node).await.unwrap();
    tx.commit().await.unwrap();

    bulk_delete(&env, &ids[1..]).await;
    reconcile(&env, &mount, &mut state).await;

    let stats = drain(&env, &mount, &mock, &mut state).await;
    assert!(stats.blocked);
    assert_eq!(mock.delete_count(), 0);
    assert_eq!(
        mock.update_count(),
        0,
        "the update is parked with the deletes, not pushed past a tripped rail"
    );
}

// ---------------------------------------------------------------------------
// 3. release
// ---------------------------------------------------------------------------

/// The block is soft and explicitly releasable — the difference between a rail
/// and an outage. The operator sets `writeback_confirm_token` to the blocked
/// batch's token and exactly that batch goes.
#[tokio::test(flavor = "multi_thread")]
async fn an_operator_confirmation_releases_exactly_the_blocked_batch() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = mirror_mount("trash");
    let mock = MirrorMock::default();

    let ids = sync_in(&mat, BULK_REVISION_THRESHOLD + 1).await;
    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;
    bulk_delete(&env, &ids).await;
    reconcile(&env, &mount, &mut state).await;

    assert!(drain(&env, &mount, &mock, &mut state).await.blocked);
    let token = state.writeback_blocked.clone().unwrap().token;

    state.writeback_confirm_token = Some(token);
    let stats = drain(&env, &mount, &mock, &mut state).await;

    assert!(!stats.blocked);
    assert_eq!(stats.deleted, ids.len());
    assert_eq!(mock.delete_count(), ids.len());
    assert!(
        state.writeback_pending_deletes.is_empty(),
        "a completed delete must leave the queue, or it is pushed on every run"
    );
    assert!(state.writeback_blocked.is_none());
    assert!(
        state.writeback_confirm_token.is_none(),
        "the confirmation is consumed, not left standing as a blanket approval"
    );

    // The policy travels with the call: `trash` is reversible and `purge` is
    // not, and an adapter told only "delete" has to guess.
    for params in mock.deletes() {
        assert_eq!(params.get("policy").and_then(|v| v.as_str()), Some("trash"));
        assert!(params.get("item_id").is_some());
    }
}

// ---------------------------------------------------------------------------
// 4. the policies
// ---------------------------------------------------------------------------

/// An ordinary hand delete, well under every rail, propagates.
///
/// The counterpart to the headline test: rails that block everything are not
/// rails, they are an off switch.
#[tokio::test(flavor = "multi_thread")]
async fn an_ordinary_delete_propagates_with_its_policy_and_concurrency_base() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = mirror_mount("purge");
    let mock = MirrorMock::default();

    let ids = sync_in(&mat, 3).await;
    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;
    bulk_delete(&env, &ids[..1]).await;
    reconcile(&env, &mount, &mut state).await;

    let stats = drain(&env, &mount, &mock, &mut state).await;
    assert!(!stats.blocked);
    assert_eq!(stats.deleted, 1);
    let call = &mock.deletes()[0];
    assert_eq!(call.get("policy").and_then(|v| v.as_str()), Some("purge"));
    assert_eq!(
        call.get("etag").and_then(|v| v.as_str()),
        Some("v1"),
        "the delete carries the pre-image's etag: nothing else still has it once \
         the node is gone, and without it a delete overwrites a remote change"
    );
    assert!(state.writeback_pending_deletes.is_empty());
}

/// `detach` is honest: the node goes locally, the remote is untouched, and the
/// next reconcile RE-IMPORTS it.
///
/// There is no per-mount suppression set anywhere in the engine, so this is
/// inherent rather than a gap — and asserting the re-import here is what stops
/// it being quietly re-described as "the delete was suppressed".
#[tokio::test(flavor = "multi_thread")]
async fn detach_pushes_nothing_and_the_item_comes_back() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = mirror_mount("detach");
    let mock = MirrorMock::default();

    let ids = sync_in(&mat, 2).await;
    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;
    bulk_delete(&env, &ids[..1]).await;
    reconcile(&env, &mount, &mut state).await;

    let stats = drain(&env, &mount, &mock, &mut state).await;
    assert_eq!(mock.delete_count(), 0, "detach must not call the provider");
    assert_eq!(stats.detached, 1);
    assert_eq!(stats.deleted, 0, "nothing was deleted anywhere but locally");
    assert!(
        state.writeback_pending_deletes.is_empty(),
        "the intent is resolved, not left to be re-evaluated forever"
    );

    // ...and the item, still at the provider, is re-imported by the next sync.
    // This is the part operators are surprised by, so it is pinned.
    let mut index = mat.load_index(&scope()).await.unwrap();
    let before = index.virtual_len();
    let mut properties = serde_json::Map::new();
    properties.insert("title".to_string(), Value::String("f000.txt".to_string()));
    mat.apply_batch(
        &scope(),
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: "f000.txt".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some("f000".to_string()),
                properties,
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: "F000".to_string(),
                etag: Some("v1".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();
    assert_eq!(
        index.virtual_len(),
        before + 1,
        "a detached node is re-imported by the next reconcile — the remote still has it"
    );
}

/// `trash` on an adapter with no trash is REFUSED, never quietly promoted to a
/// permanent delete. That substitution is the single worst thing this
/// resolution could do, so it is pinned at the mode-resolution level where the
/// console sees it too.
#[test]
fn trash_without_provider_support_refuses_the_mount_rather_than_purging() {
    let caps = Capabilities {
        supports_trash: false,
        ..mirror_caps()
    };
    let mount = mirror_mount("trash");
    match sync::write::resolve_mode(&mount.write_config, &caps, &MapperWriteback::Supported) {
        sync::write::WriteMode::Refused(r) => assert!(r.contains("supports_trash"), "{r}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    let (supported, reason) =
        sync::write::writeback_verdict(&mount.write_config, &caps, &MapperWriteback::Supported);
    assert_eq!(supported, Some(false));
    assert!(reason.unwrap().contains("supports_trash"));
}

/// Provenance, end to end (§9.3). A node under the mount path that this mount
/// does not own is never pushed as a provider delete — the pre-image has to
/// carry BOTH this mount's id and a non-empty external id.
#[tokio::test(flavor = "multi_thread")]
async fn a_delete_without_provenance_never_reaches_the_provider() {
    let env = setup().await;
    let mount = mirror_mount("purge");
    let mock = MirrorMock::default();

    let tx = begin(&env).await;
    let mut foreign = raisin_models::nodes::Node {
        id: "user-notes".to_string(),
        node_type: "raisin:Node".to_string(),
        name: "notes.txt".to_string(),
        path: format!("{MOUNT_PATH}/notes.txt"),
        workspace: Some(TARGET_WS.to_string()),
        ..Default::default()
    };
    foreign.properties.insert(
        "__mount_id".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("some-other-mount".to_string()),
    );
    foreign.properties.insert(
        "__external_id".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("SOMEONE-ELSES".to_string()),
    );
    tx.upsert_deep_node(TARGET_WS, &foreign, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;
    bulk_delete(&env, &["user-notes".to_string()]).await;
    reconcile(&env, &mount, &mut state).await;

    assert!(
        state.writeback_pending_deletes.is_empty(),
        "a node this mount never owned was queued as a provider delete"
    );
    let stats = drain(&env, &mount, &mock, &mut state).await;
    assert_eq!(mock.delete_count(), 0);
    assert_eq!(stats.deleted, 0);
}

/// A mirror's field updates use the same drain, converge check and stamp-back
/// as a `state_only` mount — not a forked second push path.
#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_pushes_field_updates_through_the_same_path() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = mirror_mount("detach");
    let mock = MirrorMock::default();

    // The scope's watched fields come from the mount, so import through one.
    let watched = MountScope {
        watched_fields: vec!["title".to_string()],
        ..scope()
    };
    let mut index = mat.load_index(&watched).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert(
        "title".to_string(),
        Value::String("original.txt".to_string()),
    );
    mat.apply_batch(
        &watched,
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: "doc.txt".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some("doc".to_string()),
                properties,
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: "D1".to_string(),
                etag: Some("v1".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();
    let node_id = index.virtual_nodes()[0].id.clone();

    let mut state = MountState::default();
    // Converged: nothing to push.
    assert_eq!(drain(&env, &mount, &mock, &mut state).await.pushed, 0);
    assert_eq!(mock.update_count(), 0);

    // A local rename diverges and is pushed exactly once — the second drain
    // sees the stamp-back and stays quiet, which is the loop terminating.
    let tx = begin(&env).await;
    let mut node = tx.get_node(TARGET_WS, &node_id).await.unwrap().unwrap();
    node.properties.insert(
        "title".to_string(),
        raisin_models::nodes::properties::PropertyValue::String("renamed.txt".to_string()),
    );
    tx.upsert_node(TARGET_WS, &node).await.unwrap();
    tx.commit().await.unwrap();

    assert_eq!(drain(&env, &mount, &mock, &mut state).await.pushed, 1);
    assert_eq!(mock.update_count(), 1);
    assert_eq!(drain(&env, &mount, &mock, &mut state).await.pushed, 0);
    assert_eq!(
        mock.update_count(),
        1,
        "a converged node must not be re-pushed"
    );
}
