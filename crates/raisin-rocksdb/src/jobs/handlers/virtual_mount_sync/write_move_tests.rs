//! Moves: the field-level kind that `move_policy` governs, and the
//! across-the-boundary kind that nothing configurable governs.
//!
//! There is no `move` operation and there is no move code path. A move is an
//! `update` carrying a location field the adapter declared, so everything in
//! part 1 is an assertion about WHICH FIELDS TRAVEL. Part 2 is the other event
//! entirely — a node leaving or entering the mount path — which the drain
//! cannot see at all, because its index is a path-prefix scan of the mount.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's environment and helpers.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as sync;
use sync::config::{MappedNode, MountConfig, MountState, WriteConfig};
use sync::materializer::{BatchOp, MountScope, VirtualMeta};
use sync::write::reconcile::reconcile_mount;
use sync::{AdapterError, AdapterInvoker, Capabilities, MapperWriteback, SyncCtx};

const LIMIT: usize = 500;

// ---------------------------------------------------------------------------
// a provider whose objects live in folders
// ---------------------------------------------------------------------------

/// An adapter that accepts two writable fields and declares that one of them —
/// `folder` — is where the object LIVES. That declaration is the whole reason
/// `move_policy` can mean anything: the engine is domain-blind and cannot tell
/// a relocation from any other property edit.
#[derive(Default)]
struct MoveMock {
    updates: Mutex<Vec<Value>>,
}

impl MoveMock {
    fn update_count(&self) -> usize {
        self.updates.lock().unwrap().len()
    }
    fn last_update(&self) -> Value {
        self.updates.lock().unwrap().last().cloned().unwrap()
    }
}

fn move_caps() -> Capabilities {
    Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        supports_changes: true,
        mutable_fields: vec!["unread".to_string(), "folder".to_string()],
        move_fields: vec!["folder".to_string()],
        ..Default::default()
    }
}

#[async_trait]
impl AdapterInvoker for MoveMock {
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
            "capabilities" => Ok(serde_json::to_value(move_caps()).unwrap()),
            "mapper_capabilities" => Ok(json!({ "to_external": true })),
            // The mapper reflects whatever the engine asked it to translate, so
            // the payload is a direct witness of the fields that travelled.
            "to_external" => {
                let props = input
                    .get("node")
                    .and_then(|n| n.get("properties"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let fields: Vec<String> = input
                    .get("fields")
                    .and_then(|f| f.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut payload = serde_json::Map::new();
                for f in fields {
                    if let Some(v) = props.get(&f) {
                        payload.insert(f, v.clone());
                    }
                }
                Ok(json!({ "payload": payload }))
            }
            "update" => {
                self.updates.lock().unwrap().push(params.clone());
                Ok(json!({ "external_id": params.get("item_id").cloned(), "etag": "v2" }))
            }
            _ => Ok(Value::Null),
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn move_mount(policy: Option<&str>) -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/mail".to_string());
    mount.write_config = WriteConfig {
        mode: "state_only".to_string(),
        mutable_fields: vec!["unread".to_string(), "folder".to_string()],
        move_policy: policy.map(str::to_string),
        ..Default::default()
    };
    mount
}

fn move_mode(mount: &MountConfig) -> sync::write::WriteMode {
    sync::write::resolve_mode(
        &mount.write_config,
        &move_caps(),
        &MapperWriteback::Supported,
    )
}

fn watching_scope() -> MountScope {
    MountScope {
        watched_fields: vec!["unread".to_string(), "folder".to_string()],
        ..scope()
    }
}

/// Materialize one message the way a sync would, so its `__pushed_state` is
/// seeded from the provider's own report rather than hand-written.
async fn sync_in_message(mat: &RocksDbMaterializer, folder: &str) -> String {
    let mut index = mat.load_index(&watching_scope()).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert("unread".to_string(), Value::Bool(false));
    properties.insert("folder".to_string(), Value::String(folder.to_string()));
    mat.apply_batch(
        &watching_scope(),
        &mut index,
        vec![BatchOp::Upsert {
            rel_path: "m1.eml".to_string(),
            mapped: MappedNode {
                node_type: "raisin:Node".to_string(),
                name: Some("m1".to_string()),
                properties,
            },
            virt: VirtualMeta {
                mount_id: MOUNT_ID.to_string(),
                external_id: "M1".to_string(),
                etag: Some("v1".to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();
    index.virtual_nodes()[0].id.clone()
}

/// A user edit: an ordinary transactional write touching no reserved property.
async fn set_props(env: &Env, node_id: &str, props: &[(&str, PropertyValue)]) {
    let tx = begin(env).await;
    let mut node = tx.get_node(TARGET_WS, node_id).await.unwrap().unwrap();
    for (k, v) in props {
        node.properties.insert(k.to_string(), v.clone());
    }
    tx.upsert_node(TARGET_WS, &node).await.unwrap();
    tx.commit().await.unwrap();
}

async fn drain(
    env: &Env,
    mount: &MountConfig,
    mock: &MoveMock,
    state: &mut MountState,
) -> sync::write::DrainStats {
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c: SyncCtx = ctx(env, mount, mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    sync::write::drain(&c, state, &mut batcher, &move_mode(mount)).await
}

// ---------------------------------------------------------------------------
// 1. `move_policy` decides which fields travel — and nothing else
// ---------------------------------------------------------------------------

/// `push`: the location field rides the ordinary `update`, alongside everything
/// else that diverged. One provider call, not two, and no new operation.
#[tokio::test(flavor = "multi_thread")]
async fn push_sends_the_location_field_in_the_same_update() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = move_mount(Some("push"));
    let mock = MoveMock::default();

    let id = sync_in_message(&mat, "inbox").await;
    set_props(
        &env,
        &id,
        &[
            ("folder", PropertyValue::String("archive".into())),
            ("unread", PropertyValue::Boolean(true)),
        ],
    )
    .await;

    let mut state = MountState::default();
    let stats = drain(&env, &mount, &mock, &mut state).await;

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.rejected, 0);
    assert_eq!(mock.update_count(), 1, "a move is ONE update, not a new op");
    let sent = mock.last_update();
    assert_eq!(sent.get("fields"), Some(&json!(["unread", "folder"])));
    assert_eq!(
        sent.get("payload"),
        Some(&json!({ "unread": true, "folder": "archive" })),
        "the location field has to reach the provider or nothing moved"
    );

    // And the baseline records it, so the same move is not pushed twice.
    assert_eq!(
        pushed_state(&node_by_id(&env, &id).await),
        json!({ "unread": true, "folder": "archive" })
    );
}

/// `detach`: the location field is withheld and the rest of the update still
/// goes. The provider keeps the object where it is; the node keeps its new
/// folder value locally.
///
/// The baseline assertion is the load-bearing one. A withheld field must not be
/// recorded as pushed — but it must also not keep re-nominating the node, which
/// is why it is dropped from the converge check as well as from the wire.
#[tokio::test(flavor = "multi_thread")]
async fn detach_withholds_the_location_field_and_pushes_the_rest() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = move_mount(Some("detach"));
    let mock = MoveMock::default();

    let id = sync_in_message(&mat, "inbox").await;
    set_props(
        &env,
        &id,
        &[
            ("folder", PropertyValue::String("archive".into())),
            ("unread", PropertyValue::Boolean(true)),
        ],
    )
    .await;

    let mut state = MountState::default();
    let stats = drain(&env, &mount, &mock, &mut state).await;

    assert_eq!(stats.pushed, 1);
    assert_eq!(mock.update_count(), 1);
    let sent = mock.last_update();
    assert_eq!(sent.get("fields"), Some(&json!(["unread"])));
    assert_eq!(
        sent.get("payload"),
        Some(&json!({ "unread": true })),
        "a detached move must not reach the provider"
    );
    assert_eq!(
        pushed_state(&node_by_id(&env, &id).await),
        json!({ "unread": true, "folder": "inbox" }),
        "the withheld move must not be recorded as pushed: the baseline keeps the \
         provider's TRUE folder (inbox), never the local value (archive). The \
         stamp extends the prior baseline rather than replacing it, so the \
         imported folder value survives the detach push."
    );

    // The local move stands — nothing moves it back — and it does not re-nominate
    // the node either.
    let node = node_by_id(&env, &id).await;
    assert_eq!(str_prop(&node, "folder").as_deref(), Some("archive"));
    let stats = drain(&env, &mount, &mock, &mut MountState::default()).await;
    assert_eq!(stats.pushed, 0, "a withheld move must not push forever");
    assert_eq!(mock.update_count(), 1);
}

/// `reject`: the WHOLE update is withheld while the location field disagrees —
/// not just the move — and the refusal is stated on the mount rather than
/// looking like a mount that is caught up.
#[tokio::test(flavor = "multi_thread")]
async fn reject_withholds_the_whole_update_and_says_so() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = move_mount(Some("reject"));
    let mock = MoveMock::default();

    let id = sync_in_message(&mat, "inbox").await;
    set_props(
        &env,
        &id,
        &[
            ("folder", PropertyValue::String("archive".into())),
            ("unread", PropertyValue::Boolean(true)),
        ],
    )
    .await;

    let mut state = MountState::default();
    let stats = drain(&env, &mount, &mock, &mut state).await;

    assert_eq!(mock.update_count(), 0, "nothing may reach the provider");
    assert_eq!(stats.rejected, 1);
    assert_eq!(stats.pushed, 0);
    assert_eq!(stats.skipped, 0, "a refusal is not a converged no-op");
    let err = state
        .writeback_last_error
        .clone()
        .expect("a refusal has to be visible on the mount");
    assert!(err.contains("move_policy"), "{err}");
    assert_eq!(state.last_drain.unwrap().rejected, 1);

    // Nothing was stamped, so the edit is still pending — and still refused.
    assert_eq!(
        pushed_state(&node_by_id(&env, &id).await),
        json!({ "unread": false, "folder": "inbox" })
    );

    // Move it back and the withheld `unread` edit goes through: the refusal
    // gates on the move, not on the node.
    set_props(
        &env,
        &id,
        &[("folder", PropertyValue::String("inbox".into()))],
    )
    .await;
    let stats = drain(&env, &mount, &mock, &mut MountState::default()).await;
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.pushed, 1);
    // `unread` alone: `reject` withholds the location field from the wire as
    // `detach` does, and once the node is back where the provider already has
    // it there is nothing about it left to send.
    assert_eq!(
        mock.last_update().get("payload"),
        Some(&json!({ "unread": true }))
    );
}

/// An adapter that declares no `move_fields` — every adapter shipped today —
/// behaves identically under every policy. Nothing about this stage can change
/// what an existing mount does.
#[tokio::test(flavor = "multi_thread")]
async fn an_adapter_without_move_fields_is_unaffected_by_the_policy() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut mount = move_mount(Some("reject"));
    mount.write_config.mutable_fields = vec!["unread".to_string(), "folder".to_string()];
    let mock = MoveMock::default();

    let id = sync_in_message(&mat, "inbox").await;
    set_props(
        &env,
        &id,
        &[("folder", PropertyValue::String("archive".into()))],
    )
    .await;

    // Resolve against capabilities that declare NO move fields.
    let caps = Capabilities {
        move_fields: Vec::new(),
        ..move_caps()
    };
    let mode = sync::write::resolve_mode(&mount.write_config, &caps, &MapperWriteback::Supported);
    let c: SyncCtx = ctx(&env, &mount, &mock, &mat);
    let mut batcher = sync::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = sync::write::drain(&c, &mut MountState::default(), &mut batcher, &mode).await;

    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.pushed, 1, "the folder is just another mutable field");
    // ONLY the diverged field travels. `unread` matches its imported baseline,
    // and a push must not carry converged fields along — some provider fields
    // have side effects on mere presence in an update (Graph re-sends meeting
    // invites whenever `attendees` appears in a PATCH).
    assert_eq!(
        mock.last_update().get("payload"),
        Some(&json!({ "folder": "archive" }))
    );
    let _ = id;
}

/// A typo in `move_policy` refuses a `state_only` mount, not only a mirror.
/// Falling back would apply a policy the operator never chose — and the one
/// they meant to type may have been `reject`.
#[test]
fn an_unrecognized_move_policy_refuses_a_state_only_mount() {
    let mount = move_mount(Some("moove"));
    match move_mode(&mount) {
        sync::write::WriteMode::Refused(r) => assert!(r.contains("move_policy"), "{r}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // And a good one resolves.
    assert!(matches!(
        move_mode(&move_mount(Some("push"))),
        sync::write::WriteMode::StateOnly(_)
    ));
}

// ---------------------------------------------------------------------------
// 2. across the mount boundary — the event the drain cannot see
// ---------------------------------------------------------------------------

/// Walk the change feed from the bottom so every commit in the test is seen.
async fn reconcile(env: &Env, mount: &MountConfig, state: &mut MountState) {
    if state.writeback_revision.is_none() {
        state.writeback_revision = Some(raisin_hlc::HLC::new(0, 0).to_string());
    }
    reconcile_mount(&env.storage, TENANT, REPO, mount, state, LIMIT)
        .await
        .unwrap();
}

/// Move a node the way a user would, creating the destination folder first.
async fn user_move(env: &Env, node_id: &str, new_path: &str) {
    let parent = &new_path[..new_path.rfind('/').unwrap()];
    if !parent.is_empty() {
        let tx = begin(env).await;
        if tx
            .get_node_by_path(TARGET_WS, parent)
            .await
            .unwrap()
            .is_none()
        {
            let folder = Node {
                id: nanoid::nanoid!(),
                node_type: "raisin:Folder".to_string(),
                name: parent.rsplit('/').next().unwrap().to_string(),
                path: parent.to_string(),
                workspace: Some(TARGET_WS.to_string()),
                ..Default::default()
            };
            tx.upsert_deep_node(TARGET_WS, &folder, "raisin:Folder")
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
    }
    let tx = begin(env).await;
    tx.move_node_tree(TARGET_WS, node_id, new_path)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// **The headline.** A mount-owned node dragged OUT of the mount path is
/// detached: it keeps its content and loses its provenance.
///
/// Before this, nothing anywhere saw the move. The node is outside the index's
/// path-prefix scan, so the drain cannot nominate it and the full reconcile
/// cannot prune it — while it still claims `__external_id`, so the provider's
/// object is re-imported next to it as a duplicate, forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_moved_out_of_the_mount_is_detached() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = move_mount(Some("push"));
    let id = sync_in_message(&mat, "inbox").await;

    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await; // walk past the import

    user_move(&env, &id, "/keepsakes/m1.eml").await;
    reconcile(&env, &mount, &mut state).await;

    assert_eq!(
        state.last_reconcile.as_ref().unwrap().detached,
        1,
        "the walk is the only thing that can see this move"
    );
    let node = node_by_id(&env, &id).await;
    assert_eq!(node.path, "/keepsakes/m1.eml");
    assert!(
        str_prop(&node, "__mount_id").is_none(),
        "a detached node must not still claim the mount"
    );
    assert!(str_prop(&node, "__external_id").is_none());
    assert!(str_prop(&node, "__etag").is_none());
    assert!(!node
        .properties
        .contains_key(sync::materializer::PUSHED_STATE_PROP));
    // The content the user moved is untouched — that is why they moved it.
    assert_eq!(str_prop(&node, "folder").as_deref(), Some("inbox"));

    // Idempotent: the detach commits as the sync actor, so the next walk skips
    // it as an echo rather than re-detaching a node it has finished with.
    reconcile(&env, &mount, &mut state).await;
    assert_eq!(state.last_reconcile.as_ref().unwrap().detached, 0);
}

/// A move WITHIN the mount is not a boundary crossing and must not detach
/// anything. The read path preserves the new path on upsert, so the local move
/// sticks — and the policy above governs the provider side, not this.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_within_the_mount_keeps_its_provenance() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = move_mount(Some("push"));
    let id = sync_in_message(&mat, "inbox").await;

    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;

    user_move(&env, &id, &format!("{MOUNT_PATH}/archive/m1.eml")).await;
    reconcile(&env, &mount, &mut state).await;

    assert_eq!(state.last_reconcile.as_ref().unwrap().detached, 0);
    let node = node_by_id(&env, &id).await;
    assert_eq!(node.path, format!("{MOUNT_PATH}/archive/m1.eml"));
    assert_eq!(str_prop(&node, "__mount_id").as_deref(), Some(MOUNT_ID));
    assert_eq!(str_prop(&node, "__external_id").as_deref(), Some("M1"));
}

/// A node dragged INTO the mount is refused, not adopted. There is no local
/// create propagation anywhere in the engine, so "adopt it and push a create"
/// is not a behaviour that exists to choose — and inventing one here would
/// upload a stranger's scratch file the moment someone tidied a folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_moved_into_the_mount_is_refused_not_adopted() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mount = move_mount(Some("push"));
    // The mount has to own something, or the walk has no reason to run.
    let _ = sync_in_message(&mat, "inbox").await;

    let tx = begin(&env).await;
    let stranger = Node {
        id: "stranger".to_string(),
        node_type: "raisin:Node".to_string(),
        name: "notes".to_string(),
        path: "/scratch/notes".to_string(),
        workspace: Some(TARGET_WS.to_string()),
        ..Default::default()
    };
    tx.upsert_deep_node(TARGET_WS, &stranger, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut state = MountState::default();
    reconcile(&env, &mount, &mut state).await;

    user_move(&env, "stranger", &format!("{MOUNT_PATH}/notes")).await;
    reconcile(&env, &mount, &mut state).await;

    assert_eq!(state.last_reconcile.as_ref().unwrap().rejected, 1);
    assert_eq!(
        state.last_reconcile.as_ref().unwrap().detached,
        0,
        "a foreign node is not this mount's to rewrite"
    );
    let node = node_by_id(&env, "stranger").await;
    assert_eq!(node.path, format!("{MOUNT_PATH}/notes"));
    assert!(
        str_prop(&node, "__mount_id").is_none(),
        "adoption would make a stranger's node a provider object"
    );
}
