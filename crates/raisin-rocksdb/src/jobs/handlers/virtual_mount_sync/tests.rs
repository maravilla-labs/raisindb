//! Sync-engine tests against a `MockAdapter` and a real RocksDB (tempdir) store.
//!
//! Covers the Phase 3 acceptance list: initial full sync, delta add/update/
//! delete, rename via `__external_id`, etag skip-write, include/exclude filters,
//! ephemeral TTL cleanup, failure backoff, no-delete of non-virtual nodes,
//! lock-held no-op exit, and stale-fencing-token rejection.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use raisin_locks::{InProcessLockManager, LockManagerHandle};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};
use raisin_storage::transactional::TransactionalStorage;
use serde_json::{json, Value};

use super::config::{MountConfig, MountState, SyncConfig, WriteConfig};
use super::materializer::{MountScope, NodeMaterializer, RocksDbMaterializer, VirtualMeta};
use super::{AdapterError, AdapterInvoker, SyncCtx, VirtualMountSyncHandler};
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "vm-test";
const TARGET_WS: &str = "default";
const MOUNT_PATH: &str = "/drive";
const MOUNT_ID: &str = "mount-mock";

// ---- test environment ----

struct Env {
    _dir: tempfile::TempDir,
    storage: Arc<RocksDBStorage>,
}

async fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());

    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    use raisin_storage::BranchRepository;
    storage
        .branches()
        .create_branch(TENANT, REPO, "main", "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(storage.clone(), TENANT, REPO, "main")
        .await
        .unwrap();
    use raisin_storage::{RepoScope, WorkspaceRepository};
    for ws in raisin_core::workspace_init::load_global_workspaces() {
        storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), ws)
            .await
            .unwrap();
    }
    Env { _dir: dir, storage }
}

use raisin_storage::{RepositoryManagementRepository, Storage};

fn scope() -> MountScope {
    MountScope {
        tenant: TENANT.to_string(),
        repo: REPO.to_string(),
        branch: "main".to_string(),
        workspace: TARGET_WS.to_string(),
        mount_id: MOUNT_ID.to_string(),
        mount_path: MOUNT_PATH.to_string(),
        force_rewrite: false,
    }
}

/// The same scope with remap semantics (etag ignored, re-path allowed).
fn remap_scope() -> MountScope {
    MountScope {
        force_rewrite: true,
        ..scope()
    }
}

/// Minimal `IntegrationConfig` carrying just an `api_config`, for snapshot tests.
fn test_integration(api_config: Value) -> super::config::IntegrationConfig {
    super::config::IntegrationConfig {
        provider_type: "test".to_string(),
        adapter_function: None,
        accounts: Vec::new(),
        api_config,
        config: Value::Null,
        connection_config_type: None,
        credential_fields: Vec::new(),
    }
}

fn mk_mount(sync: SyncConfig) -> MountConfig {
    MountConfig {
        mount_id: MOUNT_ID.to_string(),
        integration_ref: "/integrations/mock".to_string(),
        account_ref: None,
        target_workspace: TARGET_WS.to_string(),
        target_branch: "main".to_string(),
        mount_path: MOUNT_PATH.to_string(),
        remote_root: Some("root".to_string()),
        adapter_function: Some("/adapters/mock".to_string()),
        mapping_function: None,
        enabled: true,
        sync_config_raw: serde_json::to_value(&sync).unwrap(),
        sync_config: sync,
        write_config: WriteConfig::default(),
        state: MountState::default(),
    }
}

fn ext_item(id: &str, name: &str, is_folder: bool, etag: &str) -> Value {
    json!({
        "external_id": id, "name": name, "is_folder": is_folder,
        "etag": etag, "mime_type": "text/plain", "size_bytes": 10,
    })
}

// ---- mock adapter ----

enum Reply {
    Ok(Value),
    Err(AdapterError),
}

#[derive(Default)]
struct MockAdapter {
    changes: Mutex<VecDeque<Reply>>,
    lists: Mutex<HashMap<String, Value>>,
    /// Pages keyed by the CURSOR that requests them, so a paginated walk
    /// can be simulated (and a resume verified to send the right cursor).
    paged: Mutex<HashMap<String, Value>>,
    calls: Mutex<Vec<String>>,
    caps: Mutex<Option<Value>>,
    sub_reply: Mutex<Option<Value>>,
    renew_reply: Mutex<Option<Value>>,
}

impl MockAdapter {
    fn push_changes(&self, page: Value) {
        self.changes.lock().unwrap().push_back(Reply::Ok(page));
    }
    fn push_changes_err(&self, e: AdapterError) {
        self.changes.lock().unwrap().push_back(Reply::Err(e));
    }
    fn set_list(&self, folder_id: &str, page: Value) {
        self.lists
            .lock()
            .unwrap()
            .insert(folder_id.to_string(), page);
    }
    /// Page returned when `list` is called with this cursor ("" = first page).
    fn set_page(&self, cursor: &str, page: Value) {
        self.paged.lock().unwrap().insert(cursor.to_string(), page);
    }
    fn set_caps(&self, caps: Value) {
        *self.caps.lock().unwrap() = Some(caps);
    }
    fn set_sub_reply(&self, v: Value) {
        *self.sub_reply.lock().unwrap() = Some(v);
    }
    fn set_renew_reply(&self, v: Value) {
        *self.renew_reply.lock().unwrap() = Some(v);
    }
    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn saw_op(&self, op: &str) -> bool {
        self.calls.lock().unwrap().iter().any(|c| c == op)
    }
    fn op_count(&self, op: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == op)
            .count()
    }
}

#[async_trait]
impl AdapterInvoker for MockAdapter {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _adapter_path: &str,
        input: Value,
    ) -> Result<Value, AdapterError> {
        let op = input
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        self.calls.lock().unwrap().push(op.to_string());
        match op {
            "get_changes" => match self.changes.lock().unwrap().pop_front() {
                Some(Reply::Ok(v)) => Ok(v),
                Some(Reply::Err(e)) => Err(e),
                None => Ok(json!({ "items": [], "next_token": null })),
            },
            "list" => {
                let params = input.get("params");
                let cursor = params
                    .and_then(|p| p.get("cursor"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // Cursor-keyed pages win when configured; otherwise fall back to
                // the per-folder single page.
                if let Some(page) = self.paged.lock().unwrap().get(&cursor).cloned() {
                    return Ok(page);
                }
                let folder = params
                    .and_then(|p| p.get("folder_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(self
                    .lists
                    .lock()
                    .unwrap()
                    .get(&folder)
                    .cloned()
                    .unwrap_or_else(|| json!({ "items": [], "next_cursor": null })))
            }
            "capabilities" => Ok(self.caps.lock().unwrap().clone().unwrap_or(Value::Null)),
            "subscribe" => Ok(self
                .sub_reply
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Value::Null)),
            "renew" => Ok(self
                .renew_reply
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Value::Null)),
            "unsubscribe" => Ok(json!({ "ok": true })),
            _ => Ok(Value::Null),
        }
    }
}

fn ctx<'a>(
    env: &Env,
    mount: &'a MountConfig,
    invoker: &'a dyn AdapterInvoker,
    mat: &'a dyn NodeMaterializer,
) -> SyncCtx<'a> {
    SyncCtx {
        storage: env.storage.clone(),
        scope: scope(),
        config_branch: "main".to_string(),
        mount: mount.clone(),
        adapter_path: "/adapters/mock".to_string(),
        invoker,
        materializer: mat,
        lock_manager: None,
        lock_key: "k".to_string(),
        credential: None,
        mount_snapshot: super::build_mount_snapshot(mount, &test_integration(Value::Null), None),
    }
}

// ---- node inspection helpers ----

async fn all_nodes(env: &Env, ws: &str) -> Vec<Node> {
    let tx = begin(env).await;
    tx.scan_nodes(ws).await.unwrap()
}

async fn begin(env: &Env) -> Box<dyn raisin_storage::transactional::TransactionalContext> {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch("main").unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_message("test").unwrap();
    tx
}

fn str_prop(n: &Node, k: &str) -> Option<String> {
    match n.properties.get(k)? {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn virtual_assets(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|n| str_prop(n, "__mount_id").as_deref() == Some(MOUNT_ID))
        .collect()
}

// ---- tests ----

#[tokio::test(flavor = "multi_thread")]
async fn initial_full_sync_materializes_tree() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    // root: folder F + file A; folder F: file B.
    mock.set_list(
        "root",
        json!({ "items": [ext_item("F", "F", true, "e1"), ext_item("A", "A", false, "e2")], "next_cursor": null }),
    );
    mock.set_list(
        "F",
        json!({ "items": [ext_item("B", "B", false, "e3")], "next_cursor": null }),
    );

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    super::full::run(&c, &mut state).await.unwrap();

    let nodes = all_nodes(&env, TARGET_WS).await;
    let v = virtual_assets(&nodes);
    assert_eq!(v.len(), 3, "F, A, B materialized");
    assert!(v.iter().all(|n| matches!(
        n.properties.get("__virtual"),
        Some(PropertyValue::Boolean(true))
    )));
    let b = v
        .iter()
        .find(|n| str_prop(n, "__external_id").as_deref() == Some("B"))
        .unwrap();
    assert_eq!(b.path, "/drive/F/B");
    assert_eq!(b.node_type, "raisin:Node");
}

#[tokio::test(flavor = "multi_thread")]
async fn delta_add_update_delete() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    // add
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("X", "a.txt", false, "v1"), "relative_path": "a.txt" }
    ], "next_token": null }));
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    let nodes = all_nodes(&env, TARGET_WS).await;
    let n = virtual_assets(&nodes);
    assert_eq!(n.len(), 1);
    assert_eq!(str_prop(n[0], "__etag").as_deref(), Some("v1"));

    // update (new etag)
    mock.push_changes(json!({ "items": [
        { "type": "updated", "item": ext_item("X", "a.txt", false, "v2"), "relative_path": "a.txt" }
    ], "next_token": null }));
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    let nodes = all_nodes(&env, TARGET_WS).await;
    let n = virtual_assets(&nodes);
    assert_eq!(n.len(), 1, "still one node after update");
    assert_eq!(str_prop(n[0], "__etag").as_deref(), Some("v2"));

    // delete
    mock.push_changes(json!({ "items": [
        { "type": "deleted", "item": ext_item("X", "a.txt", false, "v2"), "relative_path": "a.txt" }
    ], "next_token": null }));
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_matches_by_external_id() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("X", "old", false, "v1"), "relative_path": "old" }
    ], "next_token": null }));
    let mut state = MountState::default();
    state.last_sync_token = Some("t0".to_string());
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    // Provider renames X (same external_id, new name + etag).
    mock.push_changes(json!({ "items": [
        { "type": "updated", "item": ext_item("X", "new", false, "v2"), "relative_path": "new" }
    ], "next_token": null }));
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let nodes = all_nodes(&env, TARGET_WS).await;
    let n = virtual_assets(&nodes);
    assert_eq!(n.len(), 1, "rename must not duplicate");
    assert_eq!(str_prop(n[0], "__external_id").as_deref(), Some("X"));
    assert_eq!(str_prop(n[0], "__etag").as_deref(), Some("v2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn etag_skip_write_avoids_revision() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mapped =
        super::default_mapping(&serde_json::from_value(ext_item("X", "a", false, "v1")).unwrap());
    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "X".to_string(),
        etag: Some("v1".to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };
    let wrote = mat
        .upsert(&scope(), "a", mapped.clone(), virt.clone())
        .await
        .unwrap();
    assert!(wrote, "first upsert writes");
    let again = mat.upsert(&scope(), "a", mapped, virt).await.unwrap();
    assert!(!again, "same etag must skip the write");
}

#[tokio::test(flavor = "multi_thread")]
async fn include_exclude_filters_applied() {
    let env = setup().await;
    let mut sync = SyncConfig::default();
    sync.exclude_patterns = vec!["*.tmp".to_string()];
    let mount = mk_mount(sync);
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("A", "keep.txt", false, "v1"), "relative_path": "keep.txt" },
        { "type": "created", "item": ext_item("B", "skip.tmp", false, "v1"), "relative_path": "skip.tmp" }
    ], "next_token": null }));
    let mut state = MountState::default();
    state.last_sync_token = Some("t0".to_string());
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    let nodes = all_nodes(&env, TARGET_WS).await;
    let n = virtual_assets(&nodes);
    assert_eq!(n.len(), 1);
    assert_eq!(str_prop(n[0], "__external_id").as_deref(), Some("A"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ephemeral_ttl_cleanup_removes_stale() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    // Node synced 2 hours ago.
    let old = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "X".to_string(),
        etag: Some("v1".to_string()),
        synced_at: old,
    };
    let mapped =
        super::default_mapping(&serde_json::from_value(ext_item("X", "a", false, "v1")).unwrap());
    mat.upsert(&scope(), "a", mapped, virt).await.unwrap();
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 1);

    // TTL of 1 hour → the 2-hour-old node is expired.
    let deleted = super::ephemeral::cleanup_expired(&mat, &scope(), 3600, Utc::now().timestamp())
        .await
        .unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_backoff_and_interval() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.push_changes_err(AdapterError::Transient("boom".to_string()));
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    let res = super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state).await;
    assert!(matches!(res, Err(AdapterError::Transient(_))));

    // Exponential interval backoff on consecutive failures.
    let mut m = mk_mount(SyncConfig::default()); // interval 300
    m.state.consecutive_failures = 3;
    assert_eq!(m.effective_interval_secs(), 300 * 8);
    m.state.consecutive_failures = 10; // capped at 2^5
    assert_eq!(m.effective_interval_secs(), 300 * 32);
}

/// Remap re-applies the CURRENT mapper to already-synced items.
///
/// The etag skip-write returns before the mapper's output is applied, so a
/// changed mapper — a new node type, a new folder hierarchy — is invisible to
/// everything already synced. Without a remap the only migration was
/// delete-and-reimport, which throws away node ids, history and local edits.
#[tokio::test(flavor = "multi_thread")]
async fn remap_reapplies_node_type_and_path_to_already_synced_items() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    let virt = |etag: &str| VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "M1".to_string(),
        etag: Some(etag.to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };

    // Synced by the OLD mapper: generic type, flat path.
    let old = super::config::MappedNode {
        node_type: "raisin:Node".to_string(),
        name: Some("M1".to_string()),
        properties: serde_json::from_value(json!({ "title": "Hello" })).unwrap(),
    };
    assert!(mat.upsert(&scope(), "M1", old, virt("v1")).await.unwrap());
    let before = &mat.list_virtual(&scope()).await.unwrap()[0];
    let original_id = before.id.clone();

    // The NEW mapper: different node type, and a threaded path. The provider
    // item is unchanged, so the etag is identical — an ordinary sync skips it.
    let new = || super::config::MappedNode {
        node_type: "raisin:Mail".to_string(),
        name: Some("M1".to_string()),
        properties: serde_json::from_value(json!({ "subject": "Hello" })).unwrap(),
    };
    assert!(
        !mat.upsert(&scope(), "T7/M1", new(), virt("v1"))
            .await
            .unwrap(),
        "an ordinary sync must still skip an unchanged item"
    );

    // Remap applies it.
    assert!(mat
        .upsert(&remap_scope(), "T7/M1", new(), virt("v1"))
        .await
        .unwrap());

    let after = mat.list_virtual(&scope()).await.unwrap();
    assert_eq!(after.len(), 1, "remap must not duplicate the node");
    assert_eq!(
        after[0].id, original_id,
        "remap must preserve the node id, so history and local edits survive"
    );

    let node = all_nodes(&env, TARGET_WS)
        .await
        .into_iter()
        .find(|n| n.id == original_id)
        .expect("node still present");
    assert_eq!(node.node_type, "raisin:Mail", "node type re-applied");
    assert!(
        node.path.ends_with("/T7/M1"),
        "node moved into the new hierarchy, got {}",
        node.path
    );
}

#[test]
fn path_template_groups_by_a_metadata_field() {
    let item: super::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "MSG1",
        "name": "MSG1",
        "metadata": { "conversation_id": "THREAD7", "date": "2026-08-01T09:15:00Z" },
    }))
    .unwrap();

    assert_eq!(
        super::config::resolve_path_template("{conversation_id}/{name}", &item).unwrap(),
        "THREAD7/MSG1"
    );
    assert_eq!(
        super::config::resolve_path_template("{date:%Y}/{date:%m}/{name}", &item).unwrap(),
        "2026/08/MSG1"
    );
}

/// An empty template means "keep the provider's own path" — the caller falls
/// back, so this must not return a stray empty segment.
#[test]
fn path_template_empty_means_no_hierarchy() {
    let item: super::config::ExternalItem =
        serde_json::from_value(json!({ "external_id": "A", "name": "a.txt" })).unwrap();
    assert!(super::config::resolve_path_template("", &item).is_none());
    assert!(super::config::resolve_path_template("   ", &item).is_none());
}

/// Fail CLOSED. A half-resolved template would scatter items into folders named
/// after the literal placeholder, and because the materializer preserves an
/// existing node's path, that is not undone by simply re-syncing.
#[test]
fn path_template_falls_back_when_a_placeholder_is_missing() {
    let item: super::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "A", "name": "a.txt", "metadata": { "other": "x" },
    }))
    .unwrap();
    assert!(super::config::resolve_path_template("{conversation_id}/{name}", &item).is_none());
    // Unparseable date for a formatted placeholder.
    let bad: super::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "A", "name": "a.txt", "metadata": { "date": "not-a-date" },
    }))
    .unwrap();
    assert!(super::config::resolve_path_template("{date:%Y}/{name}", &bad).is_none());
}

/// Resolved values are untrusted provider text. A `/` inside one would invent
/// hierarchy; a NUL would corrupt the storage key encoding.
#[test]
fn path_template_sanitizes_separators_inside_a_value() {
    let item: super::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "A",
        "name": "a.txt",
        "metadata": { "folder": "in/box", "nul": "a\u{0000}b" },
    }))
    .unwrap();
    let out = super::config::resolve_path_template("{folder}/{name}", &item).unwrap();
    assert_eq!(
        out, "in-box/a.txt",
        "a slash in a value must not add a level"
    );

    let out2 = super::config::resolve_path_template("{nul}/{name}", &item).unwrap();
    assert!(!out2.contains('\0'));
}

/// Empty segments collapse, so a template with a stray or leading slash still
/// yields a clean relative path.
#[test]
fn path_template_collapses_empty_segments() {
    let item: super::config::ExternalItem = serde_json::from_value(json!({
        "external_id": "A", "name": "a.txt", "metadata": { "c": "T1" },
    }))
    .unwrap();
    assert_eq!(
        super::config::resolve_path_template("/{c}//{name}/", &item).unwrap(),
        "T1/a.txt"
    );
}

/// A mailbox larger than `max_items_per_sync` must import COMPLETELY, across
/// as many runs as it takes.
///
/// Before the resumable cursor, the walk rebuilt its stack from the root and
/// started its page cursor at None on every run, so a provider with more items
/// than the cap re-imported the same first N forever and the remainder was
/// never fetched — a production-sized mailbox simply could not be synced.
#[tokio::test(flavor = "multi_thread")]
async fn backfill_resumes_across_runs_until_every_item_is_imported() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig {
        max_items_per_sync: 2,
        ..SyncConfig::default()
    });
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    // Three pages of two, two and one item, chained by cursor.
    mock.set_page(
        "",
        json!({ "items": [ext_item("A", "a", false, "v1"), ext_item("B", "b", false, "v1")],
                "next_cursor": "c1" }),
    );
    mock.set_page(
        "c1",
        json!({ "items": [ext_item("C", "c", false, "v1"), ext_item("D", "d", false, "v1")],
                "next_cursor": "c2" }),
    );
    mock.set_page(
        "c2",
        json!({ "items": [ext_item("E", "e", false, "v1")], "next_cursor": null }),
    );

    let mut state = MountState::default();

    // Run 1 — first two items, then out of budget.
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(mat.list_virtual(&scope()).await.unwrap().len(), 2);
    assert_eq!(state.backfill_cursor.as_deref(), Some("c1"));
    assert!(!state.backfill_complete, "walk is not finished yet");

    // Run 2 — resumes at c1 rather than restarting at the top.
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(mat.list_virtual(&scope()).await.unwrap().len(), 4);
    assert_eq!(state.backfill_cursor.as_deref(), Some("c2"));

    // Run 3 — final page; the walk completes and the resume point is cleared.
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert!(state.backfill_complete);
    assert!(state.backfill_cursor.is_none());
    assert!(state.backfill_stack.is_empty());

    // All five survive. The final chunk only "saw" E, so a reconcile pass here
    // would have deleted A-D — everything the backfill had just imported.
    let ids: Vec<String> = mat
        .list_virtual(&scope())
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.external_id)
        .collect();
    assert_eq!(
        ids.len(),
        5,
        "resumed backfill must not delete earlier chunks"
    );
    for want in ["A", "B", "C", "D", "E"] {
        assert!(ids.iter().any(|i| i == want), "missing {want}");
    }
}

/// An empty provider listing must NOT empty the mount by default.
///
/// The dangerous reading of "zero items" is that everything was deleted
/// upstream; the likelier one is a permissions change, a bad remote root or a
/// provider hiccup that did not raise an error. Deleted content is
/// unrecoverable, stale content is not, so the default refuses.
#[tokio::test(flavor = "multi_thread")]
async fn full_reconcile_refuses_to_empty_the_mount_on_a_zero_item_listing() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default()); // allow_empty_reconcile = false
    let mat = RocksDbMaterializer::new(env.storage.clone());

    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "X".to_string(),
        etag: Some("v1".to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };
    let mapped = super::default_mapping(
        &serde_json::from_value(ext_item("X", "synced", false, "v1")).unwrap(),
    );
    mat.upsert(&scope(), "synced", mapped, virt).await.unwrap();

    let mock = MockAdapter::default(); // empty root list
    let mut state = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let remaining = mat.list_virtual(&scope()).await.unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "an empty listing must not delete mount-owned nodes by default"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn full_reconcile_never_deletes_non_virtual_nodes() {
    let env = setup().await;
    // An empty listing normally SKIPS reconcile deletes (a provider hiccup must
    // not empty the mount). This test is specifically about who owns a node, so
    // it opts into the destructive reading to isolate that question.
    let mount = mk_mount(SyncConfig {
        allow_empty_reconcile: true,
        ..SyncConfig::default()
    });
    let mat = RocksDbMaterializer::new(env.storage.clone());

    // A user-created node under the mount path (no __mount_id).
    {
        let tx = begin(&env).await;
        let user = Node {
            id: "user-1".to_string(),
            node_type: "raisin:Node".to_string(),
            name: "mine.txt".to_string(),
            path: "/drive/mine.txt".to_string(),
            workspace: Some(TARGET_WS.to_string()),
            ..Default::default()
        };
        tx.upsert_deep_node(TARGET_WS, &user, "raisin:Folder")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    // A mount-owned node.
    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "X".to_string(),
        etag: Some("v1".to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };
    let mapped = super::default_mapping(
        &serde_json::from_value(ext_item("X", "synced", false, "v1")).unwrap(),
    );
    mat.upsert(&scope(), "synced", mapped, virt).await.unwrap();

    // Full reconcile that sees NOTHING: the virtual node is removed, the user
    // node survives.
    let mock = MockAdapter::default(); // empty root list
    let mut state = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let nodes = all_nodes(&env, TARGET_WS).await;
    assert!(
        nodes.iter().any(|n| n.id == "user-1"),
        "user node must survive reconcile"
    );
    assert_eq!(
        virtual_assets(&nodes).len(),
        0,
        "unseen virtual node removed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lock_held_makes_sync_a_no_op() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    let lock: LockManagerHandle = Arc::new(InProcessLockManager::new());
    // Another node holds the mount's lease.
    let key = format!("{TENANT}\0{REPO}\0main\0vmount:{MOUNT_ID}");
    let guard = lock
        .try_acquire(&key, "other-node", std::time::Duration::from_secs(600))
        .await
        .unwrap();
    assert!(guard.is_some());

    let mock = Arc::new(MockAdapter::default());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as super::AdapterInvokerHandle),
        Some(lock.clone()),
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "delta".to_string(),
    });
    let context = job_context();
    handler.handle(&job, &context).await.unwrap();

    // No adapter calls and no materialized nodes: it exited as a no-op.
    assert_eq!(mock.call_count(), 0, "held lock must prevent adapter calls");
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_fencing_token_rejects_state_write() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Establish a stored fencing token of 5.
    let s5 = MountState {
        last_sync_token: Some("cursor-5".to_string()),
        last_fencing_token: Some(5),
        ..Default::default()
    };
    let wrote = super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &s5)
        .await
        .unwrap();
    assert!(wrote);

    // A stale sync (token 3) must NOT overwrite.
    let s3 = MountState {
        last_sync_token: Some("cursor-3".to_string()),
        last_fencing_token: Some(3),
        ..Default::default()
    };
    let wrote = super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &s3)
        .await
        .unwrap();
    assert!(!wrote, "stale fencing token must be rejected");
    assert_eq!(read_state_token(&env).await.as_deref(), Some("cursor-5"));

    // A newer sync (token 10) is allowed.
    let s10 = MountState {
        last_sync_token: Some("cursor-10".to_string()),
        last_fencing_token: Some(10),
        ..Default::default()
    };
    let wrote = super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &s10)
        .await
        .unwrap();
    assert!(wrote);
    assert_eq!(read_state_token(&env).await.as_deref(), Some("cursor-10"));
}

#[tokio::test(flavor = "multi_thread")]
async fn null_next_token_preserves_cursor() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    // A page with items but a null next_token: the adapter is signalling "no
    // more pages", not "reset my cursor".
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("X", "a.txt", false, "v1"), "relative_path": "a.txt" }
    ], "next_token": null }));

    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    // The item was materialized, and the stored cursor is UNCHANGED (not cleared).
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 1);
    assert_eq!(
        state.last_sync_token.as_deref(),
        Some("t0"),
        "null next_token must not clear the cursor"
    );
    // The next run still takes the delta path (cursor is present).
    assert!(state.last_sync_token.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn supports_changes_false_forces_full_reconcile() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Seed a stored cursor: on cursor-presence alone the delta path would run.
    let seeded = MountState {
        last_sync_token: Some("cursor".to_string()),
        ..Default::default()
    };
    super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &seeded)
        .await
        .unwrap();

    let mock = Arc::new(MockAdapter::default());
    mock.set_caps(json!({ "can_read": true, "supports_changes": false }));
    mock.set_list(
        "root",
        json!({ "items": [ext_item("A", "a.txt", false, "v1")], "next_cursor": null }),
    );

    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as super::AdapterInvokerHandle),
        None,
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "delta".to_string(),
    });
    handler.handle(&job, &job_context()).await.unwrap();

    // Full path taken despite the stored cursor: the `list` op ran.
    assert!(
        mock.saw_op("list"),
        "supports_changes:false must force a full reconcile even with a cursor"
    );
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_of_config_branch_does_not_activate_mount() {
    let env = setup().await;
    // A forked branch carries a copy of the mount config node.
    create_extra_branch(&env, "feature").await;
    write_mount_on_branch(&env, "feature").await;

    // The periodic scan only ever reads the repo's config (default) branch,
    // which has NO mount here, so the fork's copy never enqueues a sync.
    let enqueued = super::check::run_check(
        &env.storage,
        Some(TENANT.to_string()),
        Some(REPO.to_string()),
    )
    .await
    .unwrap();
    assert_eq!(
        enqueued, 0,
        "a fork-branch mount copy must not activate a sync"
    );

    // And nothing is materialized into the fork branch.
    assert_eq!(
        virtual_assets(&all_nodes_on(&env, "feature", TARGET_WS).await).len(),
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mount_materializes_into_target_branch() {
    let env = setup().await;
    create_extra_branch(&env, "feature").await;
    // Config lives on main (config branch); mount targets the "feature" branch.
    persist_config_nodes(&env, "feature").await;

    let mock = Arc::new(MockAdapter::default());
    mock.set_list(
        "root",
        json!({ "items": [ext_item("A", "a.txt", false, "v1")], "next_cursor": null }),
    );

    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as super::AdapterInvokerHandle),
        None,
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "delta".to_string(),
    });
    handler.handle(&job, &job_context()).await.unwrap();

    // Virtual node lands in the target branch, not the config branch.
    assert_eq!(
        virtual_assets(&all_nodes_on(&env, "feature", TARGET_WS).await).len(),
        1,
        "virtual node must be materialized into the target branch"
    );
    assert_eq!(
        virtual_assets(&all_nodes(&env, TARGET_WS).await).len(),
        0,
        "config branch must stay empty"
    );
}

// ---- branch helpers ----

/// Create an additional branch and initialize its node types.
async fn create_extra_branch(env: &Env, name: &str) {
    use raisin_storage::BranchRepository;
    env.storage
        .branches()
        .create_branch(TENANT, REPO, name, "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(env.storage.clone(), TENANT, REPO, name)
        .await
        .unwrap();
}

async fn begin_on(
    env: &Env,
    branch: &str,
) -> Box<dyn raisin_storage::transactional::TransactionalContext> {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(branch).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_message("test").unwrap();
    tx
}

async fn all_nodes_on(env: &Env, branch: &str, ws: &str) -> Vec<Node> {
    let tx = begin_on(env, branch).await;
    tx.scan_nodes(ws).await.unwrap()
}

/// Write a `raisin:VirtualMount` config node directly onto `branch` (simulating
/// the copy a fork carries).
async fn write_mount_on_branch(env: &Env, branch: &str) {
    let tx = begin_on(env, branch).await;
    let mut mount = Node {
        id: MOUNT_ID.to_string(),
        node_type: "raisin:VirtualMount".to_string(),
        name: "mock".to_string(),
        path: "/mounts/mock".to_string(),
        workspace: Some(super::SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    for (k, v) in [
        ("title", "Mock"),
        ("integration_ref", "/integrations/mock"),
        ("target_workspace", TARGET_WS),
        ("mount_path", MOUNT_PATH),
        ("target_branch", branch),
    ] {
        mount
            .properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    mount
        .properties
        .insert("enabled".to_string(), PropertyValue::Boolean(true));
    tx.upsert_deep_node(super::SYSTEM_WORKSPACE, &mount, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

// ---- handler-path helpers ----

fn job_info(job_type: JobType) -> JobInfo {
    JobInfo {
        id: JobId("test-job".to_string()),
        job_type,
        status: JobStatus::Scheduled,
        tenant: TENANT.to_string(),
        started_at: Utc::now(),
        completed_at: None,
        progress: None,
        error: None,
        result: None,
        retry_count: 0,
        max_retries: 3,
        last_heartbeat: None,
        timeout_seconds: 600,
        next_retry_at: None,
        executing_since: None,
    }
}

fn job_context() -> JobContext {
    JobContext {
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: "main".to_string(),
        workspace_id: super::SYSTEM_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: HashMap::new(),
    }
}

fn prop_obj(v: Value) -> PropertyValue {
    serde_json::from_value(v).unwrap()
}

/// Persist the integration + mount config nodes into `raisin:system` on the
/// config (main) branch, with the mount materializing into `target_branch`.
async fn persist_config_nodes(env: &Env, target_branch: &str) {
    let tx = begin(env).await;

    let mut integ = Node {
        id: "integration-mock".to_string(),
        node_type: "raisin:Integration".to_string(),
        name: "mock".to_string(),
        path: "/integrations/mock".to_string(),
        workspace: Some(super::SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    integ.properties.insert(
        "title".to_string(),
        PropertyValue::String("Mock".to_string()),
    );
    integ.properties.insert(
        "provider_type".to_string(),
        PropertyValue::String("mock".to_string()),
    );
    integ.properties.insert(
        "adapter_function".to_string(),
        PropertyValue::String("/adapters/mock".to_string()),
    );
    tx.upsert_deep_node(super::SYSTEM_WORKSPACE, &integ, "raisin:Folder")
        .await
        .unwrap();

    let mut mount = Node {
        id: MOUNT_ID.to_string(),
        node_type: "raisin:VirtualMount".to_string(),
        name: "mock".to_string(),
        path: "/mounts/mock".to_string(),
        workspace: Some(super::SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    mount.properties.insert(
        "title".to_string(),
        PropertyValue::String("Mock".to_string()),
    );
    mount.properties.insert(
        "integration_ref".to_string(),
        PropertyValue::String("/integrations/mock".to_string()),
    );
    mount.properties.insert(
        "target_workspace".to_string(),
        PropertyValue::String(TARGET_WS.to_string()),
    );
    mount.properties.insert(
        "mount_path".to_string(),
        PropertyValue::String(MOUNT_PATH.to_string()),
    );
    mount.properties.insert(
        "remote_root".to_string(),
        PropertyValue::String("root".to_string()),
    );
    mount.properties.insert(
        "target_branch".to_string(),
        PropertyValue::String(target_branch.to_string()),
    );
    mount
        .properties
        .insert("enabled".to_string(), PropertyValue::Boolean(true));
    mount.properties.insert(
        "sync_config".to_string(),
        prop_obj(json!({ "mode": "poll", "interval_seconds": 300, "max_items_per_sync": 500 })),
    );
    tx.upsert_deep_node(super::SYSTEM_WORKSPACE, &mount, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[test]
fn mount_snapshot_forwards_full_sync_config_and_api_config() {
    // A mount whose sync_config carries a provider-specific key (`host`) that
    // is NOT part of the typed SyncConfig whitelist.
    let mut mount = mk_mount(SyncConfig::default());
    mount.sync_config_raw = json!({
        "mode": "poll",
        "interval_seconds": 300,
        "host": "imap.example.com",
        "port": 993,
        "tls": true,
        "mailbox": "INBOX",
    });
    let api_config = json!({ "auth_mode": "xoauth2", "default_mailbox": "INBOX" });

    let snap = super::build_mount_snapshot(&mount, &test_integration(api_config), None);

    // The full sync_config is forwarded verbatim, including the non-whitelisted key.
    let sc = snap.get("sync_config").unwrap();
    assert_eq!(
        sc.get("host").and_then(|v| v.as_str()),
        Some("imap.example.com")
    );
    assert_eq!(sc.get("port").and_then(|v| v.as_i64()), Some(993));
    assert_eq!(sc.get("mailbox").and_then(|v| v.as_str()), Some("INBOX"));
    // The integration api_config is attached.
    assert_eq!(
        snap.get("api_config")
            .unwrap()
            .get("auth_mode")
            .and_then(|v| v.as_str()),
        Some("xoauth2")
    );
    // Identity fields are still present.
    assert_eq!(
        snap.get("mount_id").and_then(|v| v.as_str()),
        Some(MOUNT_ID)
    );
    assert_eq!(
        snap.get("mount_path").and_then(|v| v.as_str()),
        Some(MOUNT_PATH)
    );
}

#[test]
fn credential_carries_connected_account_subject_as_username() {
    use super::config::ConnectedAccount;
    let account = ConnectedAccount {
        id: "acct-1".to_string(),
        subject: Some("user@example.com".to_string()),
        ..Default::default()
    };
    let tokens = json!({ "access_token": "at", "refresh_token": "rt" });
    let cred = super::build_credential("imap", &account, Some(&tokens), None, &[]);

    assert_eq!(
        cred.get("username").and_then(|v| v.as_str()),
        Some("user@example.com")
    );
    assert_eq!(
        cred.get("account_id").and_then(|v| v.as_str()),
        Some("acct-1")
    );
    assert!(cred.get("refresh_token").is_none());
}

// ---- push / webhook subscription lifecycle ----

fn webhook_mount() -> MountConfig {
    let mut sync = SyncConfig::default();
    sync.mode = "webhook".to_string();
    mk_mount(sync)
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_on_webhook_mount_stores_subscription() {
    std::env::set_var("RAISINDB_BASE_URL", "https://hook.example");
    let env = setup().await;
    let mount = webhook_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.set_sub_reply(json!({
        "subscription_id": "sub-1",
        "secret": "shh",
        "expires_at": "2999-01-01T00:00:00Z"
    }));

    let c = ctx(&env, &mount, &mock, &mat);
    let caps = super::Capabilities {
        supports_push: true,
        ..Default::default()
    };
    let mut state = MountState::default();
    super::subscription::ensure(&c, &mut state, &caps).await;

    assert_eq!(state.push_subscription_id.as_deref(), Some("sub-1"));
    assert_eq!(state.push_status.as_deref(), Some("active"));
    assert_eq!(state.push_secret.as_deref(), Some("shh"));
    assert_eq!(
        state.push_expires_at.as_deref(),
        Some("2999-01-01T00:00:00Z")
    );
    let token = state.push_mount_token.clone().expect("token generated");
    assert!(
        token.starts_with(&format!("{MOUNT_ID}.")),
        "token embeds mount_id"
    );
    assert!(state
        .push_notification_url
        .as_deref()
        .unwrap()
        .contains(&format!("/notifications/{token}")));
    assert_eq!(mock.op_count("subscribe"), 1);

    // Idempotent: a mount with a live (future-dated) subscription is not
    // re-subscribed.
    super::subscription::ensure(&c, &mut state, &caps).await;
    assert_eq!(mock.op_count("subscribe"), 1, "no re-subscribe when active");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_subscribe_when_supports_push_false() {
    let env = setup().await;
    let mount = webhook_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    let c = ctx(&env, &mount, &mock, &mat);
    let caps = super::Capabilities {
        supports_push: false,
        ..Default::default()
    };
    let mut state = MountState::default();
    super::subscription::ensure(&c, &mut state, &caps).await;

    assert!(state.push_subscription_id.is_none());
    assert_eq!(state.push_status.as_deref(), Some("unsupported"));
    assert!(!mock.saw_op("subscribe"));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_subscribe_when_mode_poll() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default()); // mode = poll
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    let c = ctx(&env, &mount, &mock, &mat);
    let caps = super::Capabilities {
        supports_push: true,
        ..Default::default()
    };
    let mut state = MountState::default();
    super::subscription::ensure(&c, &mut state, &caps).await;

    assert!(state.push_subscription_id.is_none());
    assert!(state.push_status.is_none());
    assert!(!mock.saw_op("subscribe"));
}

#[tokio::test(flavor = "multi_thread")]
async fn renew_updates_expires_at() {
    let env = setup().await;
    let mount = webhook_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.set_renew_reply(json!({
        "subscription_id": "sub-2",
        "expires_at": "2999-06-01T00:00:00Z"
    }));

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState {
        push_subscription_id: Some("sub-1".to_string()),
        push_status: Some("active".to_string()),
        push_expires_at: Some("2000-01-01T00:00:00Z".to_string()),
        push_notification_url: Some("https://hook.example/n/tok".to_string()),
        ..Default::default()
    };
    let ok = super::subscription::renew(&c, &mut state).await;

    assert!(ok);
    assert_eq!(state.push_subscription_id.as_deref(), Some("sub-2"));
    assert_eq!(
        state.push_expires_at.as_deref(),
        Some("2999-06-01T00:00:00Z")
    );
    assert_eq!(state.push_status.as_deref(), Some("active"));
    assert!(mock.saw_op("renew"));
}

#[tokio::test(flavor = "multi_thread")]
async fn unsubscribe_on_teardown() {
    let env = setup().await;
    let mount = webhook_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState {
        push_subscription_id: Some("sub-1".to_string()),
        push_status: Some("active".to_string()),
        push_secret: Some("shh".to_string()),
        push_expires_at: Some("2999-01-01T00:00:00Z".to_string()),
        ..Default::default()
    };
    super::subscription::teardown(&c, &mut state).await;

    assert!(state.push_subscription_id.is_none());
    assert!(state.push_secret.is_none());
    assert!(state.push_status.is_none());
    assert!(mock.saw_op("unsubscribe"));
}

async fn read_state_token(env: &Env) -> Option<String> {
    let tx = begin(env).await;
    let node = tx
        .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
        .await
        .unwrap()?;
    let state = serde_json::to_value(node.properties.get("state")?).ok()?;
    state
        .get("last_sync_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ---- per-connection config + credential resolution ----

/// The mount snapshot must layer all four config sources, with the mount's own
/// `sync_config` winning. `api_config` and `sync_config` stay byte-identical so
/// adapters written against them keep working.
#[test]
fn mount_snapshot_exposes_merged_config_without_disturbing_legacy_keys() {
    use raisin_models::nodes::integrations::ConnectedAccount;

    let mut mount = mk_mount(SyncConfig::default());
    mount.sync_config_raw = json!({ "mailbox": "Archive" });

    let mut integration = test_integration(json!({
        "host": "imap.default.test",
        "port": 993,
        "mailbox": "INBOX",
    }));
    integration.config = json!({ "host": "imap.connector.test" });

    let account = ConnectedAccount {
        id: "conn-2".to_string(),
        config: Some(json!({ "host": "imap.account.test", "username": "ops@example.com" })),
        ..Default::default()
    };

    let snap = super::build_mount_snapshot(&mount, &integration, Some(&account));
    let cfg = snap.get("config").unwrap();

    // Per-connection beats connector beats api_config...
    assert_eq!(
        cfg.get("host").and_then(|v| v.as_str()),
        Some("imap.account.test")
    );
    // ...the mount still wins over all of them...
    assert_eq!(cfg.get("mailbox").and_then(|v| v.as_str()), Some("Archive"));
    // ...and a key only api_config sets survives.
    assert_eq!(cfg.get("port").and_then(|v| v.as_i64()), Some(993));

    // Legacy views are untouched, so existing adapters see exactly what they did.
    assert_eq!(
        snap.get("api_config")
            .unwrap()
            .get("host")
            .and_then(|v| v.as_str()),
        Some("imap.default.test")
    );
    assert_eq!(
        snap.get("sync_config")
            .unwrap()
            .get("mailbox")
            .and_then(|v| v.as_str()),
        Some("Archive")
    );
}

/// With no connection selected there is simply no per-connection layer — this
/// is the credential-free adapter case and must not error.
#[test]
fn mount_snapshot_without_an_account_still_merges() {
    let mount = mk_mount(SyncConfig::default());
    let integration = test_integration(json!({ "host": "api.test" }));
    let snap = super::build_mount_snapshot(&mount, &integration, None);
    assert_eq!(
        snap.get("config")
            .unwrap()
            .get("host")
            .and_then(|v| v.as_str()),
        Some("api.test")
    );
}

/// A connector holding two connections must refuse to guess.
#[test]
fn account_for_refuses_to_guess_between_two_connections() {
    use raisin_models::nodes::integrations::{AccountSelectionError, ConnectedAccount};

    let mut integration = test_integration(Value::Null);
    integration.accounts = vec![
        ConnectedAccount {
            id: "a".into(),
            ..Default::default()
        },
        ConnectedAccount {
            id: "b".into(),
            ..Default::default()
        },
    ];

    assert!(matches!(
        integration.account_for(None),
        Err(AccountSelectionError::Ambiguous { .. })
    ));
    assert_eq!(integration.account_for(Some("b")).unwrap().id, "b");
    assert!(matches!(
        integration.account_for(Some("gone")),
        Err(AccountSelectionError::NotFound { .. })
    ));
}
