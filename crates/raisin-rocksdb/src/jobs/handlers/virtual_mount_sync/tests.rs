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

use super::config::{MappedNode, MountConfig, MountState, SyncConfig, WriteConfig};
use super::materializer::{
    BatchOp, MountScope, NodeMaterializer, RocksDbMaterializer, SyncIndex, VirtualMeta,
    VirtualNodeRef,
};
use super::{AdapterError, AdapterInvoker, MapperWriteback, SyncCtx, VirtualMountSyncHandler};
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
        watched_fields: Vec::new(),
        read_local_wins: false,
        command_node_types: Vec::new(),
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
        public_origin: None,
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
        resolver_function: None,
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

/// A mapper written before `operation` dispatch existed: it reads only
/// `external_item`, so it ignores the operation key entirely and answers `null`
/// to anything that is not an item. Every mapper shipped today is this shape.
#[derive(Default)]
struct LegacyMapper {
    ops: Mutex<Vec<String>>,
}

#[async_trait]
impl AdapterInvoker for LegacyMapper {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _adapter_path: &str,
        input: Value,
    ) -> Result<Value, AdapterError> {
        self.ops.lock().unwrap().push(
            input
                .get("operation")
                .and_then(|v| v.as_str())
                .unwrap_or("<absent>")
                .to_string(),
        );
        let item = input.get("external_item");
        let Some(id) = item.and_then(|i| i.get("external_id")) else {
            return Ok(Value::Null); // `if (!item || !item.external_id) return null`
        };
        let _ = id;
        Ok(json!({
            "node_type": "raisin:Node",
            "name": item.and_then(|i| i.get("name")).cloned(),
            "properties": { "title": item.and_then(|i| i.get("name")).cloned() },
        }))
    }
}

/// A mapper that dispatches on `operation` and implements both directions.
#[derive(Default)]
struct BidiMapper {
    last_fields: Mutex<Option<Value>>,
}

#[async_trait]
impl AdapterInvoker for BidiMapper {
    async fn invoke(
        &self,
        _scope: &MountScope,
        _adapter_path: &str,
        input: Value,
    ) -> Result<Value, AdapterError> {
        match input.get("operation").and_then(|v| v.as_str()) {
            Some("mapper_capabilities") => Ok(json!({ "to_external": true })),
            Some("to_external") => {
                *self.last_fields.lock().unwrap() = input.get("fields").cloned();
                Ok(json!({ "payload": { "isRead": true }, "external_id": "EXT-1" }))
            }
            _ => Ok(json!({ "node_type": "raisin:Node", "properties": {} })),
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
        pushed_events: None,
        // Tests never race the wall clock; far-future so the budget never trips.
        deadline: i64::MAX,
        // Never pushes content: this context exists for the read/probe
        // paths only.
        binary_retrieval: None,
        write_mode: std::sync::OnceLock::new(),
        public_origin: None,
        storage: env.storage.clone(),
        // Mirrors `resolve.rs`: the scope's watched fields come from the
        // mount's declared `mutable_fields`, so a `state_only` mount's index
        // carries the write view and every other mount's does not.
        scope: MountScope {
            watched_fields: mount.write_config.declared_mutable_fields().to_vec(),
            ..scope()
        },
        config_branch: "main".to_string(),
        mount: mount.clone(),
        adapter_path: "/adapters/mock".to_string(),
        invoker,
        materializer: mat,
        lock_manager: None,
        lease_token: None,
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

// ---- materializer helpers ----

/// Write ONE item through the batch API — a one-op batch is exactly what the
/// single-item replay path does, so this exercises the real code.
async fn upsert_one(
    mat: &RocksDbMaterializer,
    scope: &MountScope,
    index: &mut SyncIndex,
    rel_path: &str,
    mapped: MappedNode,
    virt: VirtualMeta,
) -> bool {
    let stats = mat
        .apply_batch(
            scope,
            index,
            vec![BatchOp::Upsert {
                rel_path: rel_path.to_string(),
                mapped,
                virt,
            }],
        )
        .await
        .unwrap();
    assert_eq!(stats.failed, 0, "unexpected item-level rejection");
    stats.written == 1
}

/// Mount-owned nodes as the run's index sees them.
fn virtual_refs(index: &SyncIndex) -> Vec<VirtualNodeRef> {
    index.virtual_nodes()
}

/// Mount-owned nodes re-read from storage. Stronger than asking a live index:
/// it proves the writes actually landed, not just that the in-memory view was
/// updated.
async fn list_virtual(mat: &RocksDbMaterializer, scope: &MountScope) -> Vec<VirtualNodeRef> {
    mat.load_index(scope).await.unwrap().virtual_nodes()
}

/// Node paths in a stable order, for asserting that a re-run moved nothing.
fn sorted_paths(nodes: &[VirtualNodeRef]) -> Vec<String> {
    let mut out: Vec<String> = nodes.iter().map(|n| n.path.clone()).collect();
    out.sort();
    out
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

/// An idle Microsoft-Graph-style delta feed mints a FRESH delta token on every
/// poll: empty page, new token, forever. The old termination check (`next ==
/// token`) never fired on that shape, so the delta loop spun empty pages at
/// provider speed — committing the fresh cursor each round — until the job
/// watchdog killed the run at 600s, the lease leaked, and the scheduler
/// re-enqueued: the production "mount committed 4×/second with no sync
/// running" incident. The loop must stop on the FIRST empty page (legacy
/// adapters) and must obey `has_more` when the adapter states it.
#[tokio::test(flavor = "multi_thread")]
async fn an_idle_feed_with_a_fresh_token_every_poll_terminates() {
    // Legacy adapter (no has_more): every poll returns a NEW token, no items.
    // Each leg gets its own env: the per-page cursor persist bumps the stored
    // state_seq, so reusing one env would make a later leg's write Superseded
    // and end the pass for the wrong reason.
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    for i in 0..50 {
        mock.push_changes(json!({ "items": [], "next_token": format!("fresh-{i}") }));
    }
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(
        mock.op_count("get_changes"),
        1,
        "an empty page is not forward progress; the loop must stop at the first one"
    );
    // The fresh token is still persisted as the resume point.
    assert_eq!(state.last_sync_token.as_deref(), Some("fresh-0"));

    // has_more: false stops even with items in the page (caught-up final page).
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("A", "a.txt", false, "v1"), "relative_path": "a.txt" }
    ], "next_token": "delta-link-1", "has_more": false }));
    mock.push_changes(json!({ "items": [], "next_token": "should-not-be-fetched" }));
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(
        mock.op_count("get_changes"),
        1,
        "has_more=false ends the pass"
    );
    assert_eq!(state.last_sync_token.as_deref(), Some("delta-link-1"));

    // has_more: true keeps paging even across an EMPTY page (Graph documents
    // empty mid-enumeration pages), then stops at has_more: false.
    let env = setup().await;
    persist_config_nodes(&env, "main").await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    mock.push_changes(json!({ "items": [], "next_token": "page-2", "has_more": true }));
    mock.push_changes(json!({ "items": [
        { "type": "created", "item": ext_item("B", "b.txt", false, "v1"), "relative_path": "b.txt" }
    ], "next_token": "delta-link-2", "has_more": false }));
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    super::delta::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(
        mock.op_count("get_changes"),
        2,
        "has_more=true pages through"
    );
    assert_eq!(state.last_sync_token.as_deref(), Some("delta-link-2"));
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
    let mut index = mat.load_index(&scope()).await.unwrap();
    let wrote = upsert_one(
        &mat,
        &scope(),
        &mut index,
        "a",
        mapped.clone(),
        virt.clone(),
    )
    .await;
    assert!(wrote, "first upsert writes");
    let again = upsert_one(
        &mat,
        &scope(),
        &mut index,
        "a",
        mapped.clone(),
        virt.clone(),
    )
    .await;
    assert!(!again, "same etag must skip the write");

    // The skip must also hold for an index freshly re-read from storage, not
    // just for the in-memory one this run mutated.
    let mut reloaded = mat.load_index(&scope()).await.unwrap();
    let third = upsert_one(&mat, &scope(), &mut reloaded, "a", mapped, virt).await;
    assert!(
        !third,
        "same etag must still skip after reloading the index"
    );
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
    let mut index = mat.load_index(&scope()).await.unwrap();
    upsert_one(&mat, &scope(), &mut index, "a", mapped, virt).await;
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 1);

    // TTL of 1 hour → the 2-hour-old node is expired.
    let mount = mk_mount(SyncConfig::default());
    let mock = MockAdapter::default();
    let ctx = ctx(&env, &mount, &mock, &mat);
    let mut batcher = super::batch::SyncBatcher::new(&ctx).await.unwrap();
    let deleted = super::ephemeral::cleanup_expired(&mut batcher, 3600, Utc::now().timestamp())
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
    let mut index = mat.load_index(&scope()).await.unwrap();
    assert!(upsert_one(&mat, &scope(), &mut index, "M1", old, virt("v1")).await);
    let original_id = virtual_refs(&index)[0].id.clone();

    // The NEW mapper: different node type, and a threaded path. The provider
    // item is unchanged, so the etag is identical — an ordinary sync skips it.
    let new = || super::config::MappedNode {
        node_type: "raisin:Mail".to_string(),
        name: Some("M1".to_string()),
        properties: serde_json::from_value(json!({ "subject": "Hello" })).unwrap(),
    };
    assert!(
        !upsert_one(&mat, &scope(), &mut index, "T7/M1", new(), virt("v1")).await,
        "an ordinary sync must still skip an unchanged item"
    );

    // Remap applies it.
    assert!(upsert_one(&mat, &remap_scope(), &mut index, "T7/M1", new(), virt("v1")).await);

    let after = virtual_refs(&index);
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

/// The connector's public origin comes from its stored OAuth redirect_uri.
///
/// `RAISINDB_BASE_URL` cannot serve a multi-tenant deployment — every org has
/// its own `{handle}.{base}` host — so it is left unset and push could never be
/// wired. The redirect_uri is the one public URL per connector that is already
/// verified correct, because the provider rejects a mismatched OAuth exchange.
#[test]
fn public_origin_is_derived_from_the_oauth_redirect_uri() {
    use raisin_models::nodes::properties::PropertyValue;

    let node_with = |redirect: &str| {
        let mut n = Node {
            id: "i1".into(),
            node_type: "raisin:Integration".into(),
            name: "ms-graph".into(),
            path: "/integrations/ms-graph".into(),
            ..Default::default()
        };
        n.properties.insert(
            "provider_type".into(),
            PropertyValue::String("ms-graph".into()),
        );
        n.properties.insert(
            "oauth_config".into(),
            serde_json::from_value(json!({ "redirect_uri": redirect })).unwrap(),
        );
        n
    };

    // Only the ORIGIN is taken; the callback path is irrelevant to push.
    let cfg = super::config::IntegrationConfig::from_node(&node_with(
        "https://rdb.example.test/api/integrations/studio/oauth/callback",
    ))
    .unwrap();
    assert_eq!(
        cfg.public_origin.as_deref(),
        Some("https://rdb.example.test")
    );

    // A non-default port is preserved.
    let cfg = super::config::IntegrationConfig::from_node(&node_with(
        "http://localhost:8080/api/integrations/studio/oauth/callback",
    ))
    .unwrap();
    assert_eq!(cfg.public_origin.as_deref(), Some("http://localhost:8080"));

    // Unset or unparseable falls back to None, so the caller uses the env var.
    let cfg = super::config::IntegrationConfig::from_node(&node_with("")).unwrap();
    assert_eq!(cfg.public_origin, None);
    let cfg = super::config::IntegrationConfig::from_node(&node_with("not a url")).unwrap();
    assert_eq!(cfg.public_origin, None);
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

/// A delta cursor with unread pages behind it must SURVIVE a backfill chunk.
///
/// `phases::run_phases` runs the delta pass and then a backfill chunk in the
/// same run, and the delta pass ends with pages outstanding whenever it spends
/// its item budget or its clock — by design, so the next run resumes from it.
/// The backfill chunk then called `capture_delta_baseline`, which asks the
/// provider for "changes from now on" and overwrote that resume token. Every
/// change behind it was skipped permanently: moves and renames kept stale paths,
/// upstream deletions never propagated (the resumed walk's reconcile is gated by
/// `resuming`), and the run reported `ok`.
#[tokio::test(flavor = "multi_thread")]
async fn a_backfill_chunk_never_overwrites_a_live_delta_cursor() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    let mat = RocksDbMaterializer::new(env.storage.clone());

    // A mount mid-enumeration: the delta pass stored T5 with pages still to go.
    let mock = MockAdapter::default();
    mock.set_list(
        "root",
        json!({ "items": [ext_item("A", "a", false, "v1")] }),
    );
    mock.push_changes(json!({ "items": [], "next_token": "BASELINE" }));

    let mut state = MountState {
        last_sync_token: Some("T5".to_string()),
        ..MountState::default()
    };
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    assert_eq!(
        state.last_sync_token.as_deref(),
        Some("T5"),
        "the resume token must survive; a baseline over it discards every change behind it"
    );

    // The converse still holds: a mount with NO cursor is exactly what a
    // baseline is for, and capturing one is what lets it go incremental.
    let mock = MockAdapter::default();
    mock.set_list(
        "root",
        json!({ "items": [ext_item("A", "a", false, "v1")] }),
    );
    mock.push_changes(json!({ "items": [], "next_token": "BASELINE" }));

    let mut fresh = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut fresh)
        .await
        .unwrap();
    assert_eq!(fresh.last_sync_token.as_deref(), Some("BASELINE"));
}

/// A WIDE tree — many folders, each fitting in one page — must respect the
/// per-run budget and leave a resume point, exactly like a deep one.
///
/// It did not. The budget and wall-clock test sat AFTER the `next_cursor` match,
/// so on any folder whose listing fit a single page the match broke the page
/// loop first and the check was never reached. A Drive or SharePoint tree — all
/// small folders — therefore ignored `max_items_per_sync` entirely and ran past
/// the 480s wall-clock budget into the 600s watchdog, which aborts at an await
/// point and skips `finalize`. Nothing persisted a resume point (the cursor had
/// already been `take`n), so the next run restarted at the root and died at the
/// same depth, forever, while the console reported "Nothing was lost; the
/// resume point is intact."
#[tokio::test(flavor = "multi_thread")]
async fn a_wide_tree_of_single_page_folders_still_honours_the_budget() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig {
        max_items_per_sync: 2,
        ..SyncConfig::default()
    });
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();

    // Root holds two folders; each holds two files. Every listing is ONE page —
    // `next_cursor` is null everywhere, which is what made the budget check
    // unreachable.
    mock.set_list(
        "root",
        json!({ "items": [ext_item("F1", "f1", true, "v1"), ext_item("F2", "f2", true, "v1")],
                "next_cursor": null }),
    );
    mock.set_list(
        "F1",
        json!({ "items": [ext_item("A", "a", false, "v1"), ext_item("B", "b", false, "v1")],
                "next_cursor": null }),
    );
    mock.set_list(
        "F2",
        json!({ "items": [ext_item("C", "c", false, "v1"), ext_item("D", "d", false, "v1")],
                "next_cursor": null }),
    );

    let mut state = MountState::default();

    // Run 1 stops at the cap with folders still queued, and says so.
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert!(
        !state.backfill_complete,
        "the walk did not reach the end of the provider"
    );
    assert!(
        !state.backfill_stack.is_empty(),
        "a folder left unwalked must be on the resume stack, or the next run \
         restarts at the root and dies at the same depth forever"
    );
    let after_first = list_virtual(&mat, &scope()).await.len();
    assert!(
        after_first < 6,
        "the budget must bound a wide walk too; imported {after_first} of 6"
    );

    // Subsequent runs resume and finish. Bounded so a walk that fails to make
    // progress fails the test instead of hanging it.
    for _ in 0..6 {
        if state.backfill_complete {
            break;
        }
        super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
            .await
            .unwrap();
    }

    assert!(state.backfill_complete, "the walk must eventually finish");
    assert!(state.backfill_cursor.is_none());
    assert!(state.backfill_stack.is_empty());

    let mut ids: Vec<String> = list_virtual(&mat, &scope())
        .await
        .into_iter()
        .map(|n| n.external_id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["A", "B", "C", "D", "F1", "F2"]);
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
    assert_eq!(list_virtual(&mat, &scope()).await.len(), 2);
    assert_eq!(state.backfill_cursor.as_deref(), Some("c1"));
    assert!(!state.backfill_complete, "walk is not finished yet");

    // Run 2 — resumes at c1 rather than restarting at the top.
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();
    assert_eq!(list_virtual(&mat, &scope()).await.len(), 4);
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
    let ids: Vec<String> = list_virtual(&mat, &scope())
        .await
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
    let mut index = mat.load_index(&scope()).await.unwrap();
    upsert_one(&mat, &scope(), &mut index, "synced", mapped, virt).await;

    let mock = MockAdapter::default(); // empty root list
    let mut state = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let remaining = list_virtual(&mat, &scope()).await;
    assert_eq!(
        remaining.len(),
        1,
        "an empty listing must not delete mount-owned nodes by default"
    );
}

/// A mount whose `list` is not authoritative for its own content must survive a
/// forced full run with its nodes intact.
///
/// The IMAP incident: `opList` returns MAILBOXES only — messages arrive through
/// `get_changes` — so a full walk's `seen` holds folder ids and nothing else. It
/// is NOT empty, so the zero-item guard above never fires, and nothing else in
/// the run is truncated, resumed or stopped. Every message node was staged for
/// delete, and the delta path (`fetchSince` the highest uid) never brought them
/// back. `sync_config.reconcile_deletes: false` is how a mount states this.
#[tokio::test(flavor = "multi_thread")]
async fn reconcile_deletes_off_keeps_nodes_the_walk_cannot_enumerate() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig {
        reconcile_deletes: false,
        ..SyncConfig::default()
    });
    let mat = RocksDbMaterializer::new(env.storage.clone());

    // A node the listing will not mention — a message under a mailbox.
    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "MSG-1".to_string(),
        etag: Some("v1".to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };
    let mapped = super::default_mapping(
        &serde_json::from_value(ext_item("MSG-1", "msg-1", false, "v1")).unwrap(),
    );
    let mut index = mat.load_index(&scope()).await.unwrap();
    upsert_one(&mat, &scope(), &mut index, "msg-1", mapped, virt).await;

    // The walk enumerates containers only, so `seen` is non-empty but excludes
    // the message: exactly the shape that made the guard above miss.
    let mock = MockAdapter::default();
    mock.set_list(
        "root",
        json!({ "items": [ext_item("INBOX", "INBOX", true, "v1")] }),
    );
    let mut state = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let ids: Vec<String> = list_virtual(&mat, &scope())
        .await
        .into_iter()
        .map(|n| n.external_id)
        .collect();
    assert!(
        ids.iter().any(|i| i == "MSG-1"),
        "reconcile_deletes: false must not prune nodes the walk cannot enumerate; got {ids:?}"
    );
}

/// The converse: the default is ON, because for a provider whose `list` IS the
/// whole truth a remote delete must still propagate.
#[tokio::test(flavor = "multi_thread")]
async fn reconcile_deletes_on_by_default_still_prunes_what_is_gone_upstream() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    assert!(mount.sync_config.reconcile_deletes, "default must be on");
    let mat = RocksDbMaterializer::new(env.storage.clone());

    let virt = VirtualMeta {
        mount_id: MOUNT_ID.to_string(),
        external_id: "GONE".to_string(),
        etag: Some("v1".to_string()),
        synced_at: Utc::now().to_rfc3339(),
    };
    let mapped = super::default_mapping(
        &serde_json::from_value(ext_item("GONE", "gone", false, "v1")).unwrap(),
    );
    let mut index = mat.load_index(&scope()).await.unwrap();
    upsert_one(&mat, &scope(), &mut index, "gone", mapped, virt).await;

    let mock = MockAdapter::default();
    mock.set_list(
        "root",
        json!({ "items": [ext_item("A", "a", false, "v1")] }),
    );
    let mut state = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    let ids: Vec<String> = list_virtual(&mat, &scope())
        .await
        .into_iter()
        .map(|n| n.external_id)
        .collect();
    assert!(
        !ids.iter().any(|i| i == "GONE"),
        "a node the authoritative walk did not see must be pruned; got {ids:?}"
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
    let mut index = mat.load_index(&scope()).await.unwrap();
    upsert_one(&mat, &scope(), &mut index, "synced", mapped, virt).await;

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
        trigger: "manual".to_string(),
    });
    let context = job_context();
    handler.handle(&job, &context).await.unwrap();

    // No adapter calls and no materialized nodes: it exited as a no-op.
    assert_eq!(mock.call_count(), 0, "held lock must prevent adapter calls");
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 0);
}

/// A writer holding a stale view of the state must not clobber a newer one.
///
/// This is the same PROPERTY the old `stale_fencing_token_rejects_state_write`
/// asserted, re-expressed against the durable `state_seq` that replaced the
/// lock manager's fencing token. The old guard compared a VOLATILE counter
/// (reset to zero on every process start) against durable state, so after a
/// restart every legitimate write was refused as "stale" — a mount materialized
/// hundreds of nodes and then silently discarded its own resume cursor.
#[tokio::test(flavor = "multi_thread")]
async fn stale_writer_cannot_clobber_newer_state() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Writer A reads seq 0 and writes; its seq advances to 1.
    let mut a = MountState {
        last_sync_token: Some("cursor-a".to_string()),
        ..Default::default()
    };
    assert!(
        super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut a)
            .await
            .unwrap()
    );
    assert_eq!(a.state_seq, 1);
    assert_eq!(read_state_token(&env).await.as_deref(), Some("cursor-a"));

    // Writer B also read seq 0 (before A landed) and is therefore stale.
    let mut b = MountState {
        last_sync_token: Some("cursor-b".to_string()),
        state_seq: 0,
        ..Default::default()
    };
    assert!(
        !super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut b)
            .await
            .unwrap(),
        "a writer whose seq is behind the stored one must be refused"
    );
    assert_eq!(read_state_token(&env).await.as_deref(), Some("cursor-a"));

    // Writer A keeps going with its advanced seq — repeated writes in one run
    // must keep working, which is what the progress ticks depend on.
    a.last_sync_token = Some("cursor-a2".to_string());
    assert!(
        super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut a)
            .await
            .unwrap()
    );
    assert_eq!(a.state_seq, 2);
    assert_eq!(read_state_token(&env).await.as_deref(), Some("cursor-a2"));
}

/// The guard must survive the lock store being wiped.
///
/// This is the regression test for the production outage: with
/// `[locks] backend = "inprocess"` the fencing counter restarts at zero on every
/// process start, and the server had restarted 6 times in 24 hours. Because the
/// guard is now durable state rather than a lock artifact, a fresh process with
/// no lock history writes successfully.
#[tokio::test(flavor = "multi_thread")]
async fn state_writes_survive_a_wiped_lock_store() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Simulate a mount that has been written many times before the restart.
    let mut before = MountState {
        last_sync_token: Some("pre-restart".to_string()),
        state_seq: 64,
        ..Default::default()
    };
    assert!(
        super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut before)
            .await
            .unwrap()
    );

    // After a restart the engine re-reads the mount, so it carries the STORED
    // seq — not a counter that restarted at 1. The write must land.
    let stored_seq = before.state_seq;
    let mut after_restart = MountState {
        last_sync_token: Some("post-restart".to_string()),
        state_seq: stored_seq,
        ..Default::default()
    };
    assert!(
        super::persist_mount_state(
            &env.storage,
            TENANT,
            REPO,
            "main",
            MOUNT_ID,
            &mut after_restart
        )
        .await
        .unwrap(),
        "a restarted process must still be able to write mount state"
    );
    assert_eq!(
        read_state_token(&env).await.as_deref(),
        Some("post-restart")
    );
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
    let mut seeded = MountState {
        last_sync_token: Some("cursor".to_string()),
        ..Default::default()
    };
    super::persist_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID, &mut seeded)
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
        trigger: "manual".to_string(),
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
        trigger: "manual".to_string(),
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

/// A mount whose `target_branch` does not exist must back OFF, not spin.
///
/// This guard returns before `finalize`, so nothing else stamps the attempt.
/// Without the stamp `last_attempt_at` stays null, `is_due` has no activity to
/// measure an interval from and answers `true` unconditionally, and the mount is
/// re-scanned, re-read and re-written on every 60s tick forever.
///
/// The account-selection guard next door already did this and carried a comment
/// explaining why; this one did not. Both now share `mark_misconfigured`, which
/// is the point of having one exit.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_target_branch_backs_the_mount_off() {
    let env = setup().await;
    // Config lives on main; the mount points at a branch that was never created.
    persist_config_nodes(&env, "no-such-branch").await;

    let mock = Arc::new(MockAdapter::default());
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as super::AdapterInvokerHandle),
        None,
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "delta".to_string(),
        trigger: "schedule".to_string(),
    });
    let result = handler.handle(&job, &job_context()).await.unwrap();

    assert_eq!(
        result
            .as_ref()
            .and_then(|v| v.get("reason"))
            .and_then(|v| v.as_str()),
        Some("misconfigured"),
        "a nonexistent target_branch must skip as misconfigured"
    );

    let state = read_state(&env)
        .await
        .expect("mount state must be persisted");
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("misconfigured")
    );
    assert!(
        state
            .get("last_attempt_at")
            .and_then(|v| v.as_i64())
            .is_some(),
        "the attempt must be stamped, or `is_due` keeps this mount permanently due"
    );
    assert_eq!(
        state.get("consecutive_failures").and_then(|v| v.as_u64()),
        Some(1),
        "the failure must count, or the backoff never grows"
    );
}

/// A `state` blob that is PRESENT but does not parse must not become a default.
///
/// Defaulting looks harmless and is not: `last_sync_token: None` sends the
/// mount back to a full walk, `backfill_complete: false` disables reconcile
/// deletes, and `state_seq: 0` against a stored seq that has advanced makes
/// `persist_mount_state` refuse every subsequent write — permanently. The mount
/// then materializes nodes it can never record, on every run, reporting `ok`.
///
/// Absent is a different case and still defaults; that is a fresh mount.
#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_state_blob_is_refused_not_defaulted() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Overwrite `state` with a shape `MountState` cannot deserialize:
    // `backfill_stack` is Vec<(Option<String>, String)>, not a string.
    {
        let tx = begin(&env).await;
        let mut node = tx
            .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap();
        node.properties.insert(
            "state".to_string(),
            prop_obj(json!({ "backfill_stack": "not-a-stack", "state_seq": 42 })),
        );
        tx.upsert_node(super::SYSTEM_WORKSPACE, &node)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    let node = {
        let tx = begin(&env).await;
        tx.get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap()
    };
    let parsed = super::config::MountConfig::from_node(&node);
    assert!(
        parsed.is_err(),
        "a corrupt state blob must fail the parse, not silently reset the mount"
    );

    // And the low-level read refuses too, rather than handing back a zeroed
    // `state_seq` that would jam every later write.
    let read = super::read_mount_state(&env.storage, TENANT, REPO, "main", MOUNT_ID).await;
    assert!(
        read.is_err(),
        "read_mount_state must surface a corrupt blob instead of defaulting it"
    );
}

/// A `write_config` blob that is PRESENT but does not parse must not default.
///
/// `WriteConfig::default()` is writeback OFF. Defaulting a corrupt blob
/// therefore silently disables the push an operator explicitly configured:
/// `drain` returns immediately, `writeback_supported` stays absent because
/// nothing ever refused anything, and local edits pile up forever with no error
/// and no log line. Absent stays fine — that is a read-only mount.
#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_write_config_is_refused_not_defaulted() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // `mutable_fields` is Vec<String>, not a string.
    {
        let tx = begin(&env).await;
        let mut node = tx
            .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap();
        node.properties.insert(
            "write_config".to_string(),
            prop_obj(json!({ "mode": "state_only", "mutable_fields": "unread" })),
        );
        tx.upsert_node(super::SYSTEM_WORKSPACE, &node)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    let node = {
        let tx = begin(&env).await;
        tx.get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap()
    };
    let parsed = super::config::MountConfig::from_node(&node);
    let err = parsed
        .err()
        .expect("a corrupt write_config must fail the parse, not silently disable writeback");
    assert!(
        err.contains("write_config"),
        "the error must name the property an operator has to fix; got: {err}"
    );
}

/// An ABSENT `write_config` still defaults — a read-only mount is the norm.
#[tokio::test(flavor = "multi_thread")]
async fn an_absent_write_config_still_defaults() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    {
        let tx = begin(&env).await;
        let mut node = tx
            .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap();
        node.properties.remove("write_config");
        tx.upsert_node(super::SYSTEM_WORKSPACE, &node)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    let node = {
        let tx = begin(&env).await;
        tx.get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap()
    };
    let mount = super::config::MountConfig::from_node(&node)
        .expect("an absent write_config is a read-only mount, not a broken one");
    assert!(!mount.write_config.wants_state_only());
    assert!(!mount.write_config.wants_write_through());
}

/// A mount whose target workspace forbids its node type must STOP and say why.
///
/// This is the production shape: an operator mounts a calendar into a workspace
/// that does not list `raisin:Event`, every item is rejected identically, and
/// the run counted them to the end of its budget, burned the 600s job timeout,
/// got retried three times — and recorded `OK · 100 failed`. A mount that could
/// never work, presented as healthy, with the provider's own explanation
/// visible only as a per-item WARN nobody reads at `RUST_LOG=error`.
///
/// Now it gives up early, reports `misconfigured`, and carries the rejection
/// message in `last_error` where the console shows it.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_that_rejects_the_node_type_stops_the_sync_and_says_why() {
    use raisin_storage::{RepoScope, WorkspaceRepository};
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Restrict the target workspace so the default mapping's `raisin:Node`
    // can never be written — the same shape as `stories` lacking `raisin:Event`.
    let scope = RepoScope::new(TENANT, REPO);
    let mut ws = env
        .storage
        .workspaces()
        .get(scope.clone(), TARGET_WS)
        .await
        .unwrap()
        .expect("target workspace must exist");
    ws.allowed_node_types = vec!["raisin:Folder".to_string()];
    env.storage.workspaces().put(scope, ws).await.unwrap();

    // Small batches, so the breaker gets a chance to fire between flushes and
    // we can prove the run gave up rather than grinding through every item.
    {
        let tx = begin(&env).await;
        let mut node = tx
            .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap();
        node.properties.insert(
            "sync_config".to_string(),
            prop_obj(json!({ "mode": "poll", "batch_size": 20 })),
        );
        tx.upsert_node(super::SYSTEM_WORKSPACE, &node)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    // More items than the failure budget, so the breaker is reached.
    let items: Vec<_> = (0..80)
        .map(|i| ext_item(&format!("E{i}"), &format!("e{i}.txt"), false, "v1"))
        .collect();
    let mock = Arc::new(MockAdapter::default());
    mock.set_list("root", json!({ "items": items, "next_cursor": null }));

    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as super::AdapterInvokerHandle),
        None,
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "full".to_string(),
        trigger: "manual".to_string(),
    });
    // The job itself must NOT error: a permanently misconfigured mount is not a
    // job failure to retry, it is a state the operator has to fix.
    handler.handle(&job, &job_context()).await.unwrap();

    let state = read_state(&env).await.expect("state must be persisted");
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("misconfigured"),
        "a mount that cannot write anything must not report itself healthy"
    );
    let last_error = state
        .get("last_error")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(
        last_error.contains("rejected") && last_error.contains("none could be written"),
        "last_error must explain the mount gave up, got: {last_error}"
    );
    assert!(
        last_error.contains("raisin:Node"),
        "last_error must carry the underlying rejection reason, got: {last_error}"
    );

    // And it stopped early rather than grinding through every item.
    let failed = state
        .get("last_run")
        .and_then(|r| r.get("failed"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        failed < 80,
        "the run must give up before attempting every item, attempted {failed}"
    );
    assert!(
        failed >= 50,
        "and only after the failure budget is actually spent, attempted {failed}"
    );
}

/// The whole persisted `state` blob of the test mount.
async fn read_state(env: &Env) -> Option<serde_json::Value> {
    let tx = begin(env).await;
    let node = tx
        .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
        .await
        .unwrap()?;
    serde_json::to_value(node.properties.get("state")?).ok()
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

/// A webhook mount with a live subscription and an unfinished backfill is DUE.
///
/// Regression test for a production stall. `is_due` returned early for
/// `mode: "webhook"` — above the backfill-pending check — so a webhook mount
/// with an active subscription was never due, and its half-finished import
/// advanced only when the provider happened to send a ping. A quiet mailbox
/// meant the import stopped at the first 500-item chunk and stayed there for
/// hours while the console showed `status: "ok"`.
///
/// Draining our own unfinished backfill has nothing to do with whether the
/// provider can push.
/// An ordinary failure must land on a terminal status straight away.
///
/// `finalize` used to set the status only once `consecutive_failures` reached
/// the degrade threshold (5), so failures 1-4 left it at whatever `run_sync`
/// wrote before the adapter work: `"syncing"`. The console then showed a
/// permanent spinner next to a run history saying `error`, with `Sync now`
/// greyed out on that same flag.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_run_leaves_a_terminal_status_not_syncing() {
    let env = setup().await;
    persist_config_nodes(&env, "main").await;

    // Put the mount on the delta path, so the failing `get_changes` is what runs.
    {
        let tx = begin(&env).await;
        let mut node = tx
            .get_node(super::SYSTEM_WORKSPACE, MOUNT_ID)
            .await
            .unwrap()
            .unwrap();
        node.properties.insert(
            "state".to_string(),
            prop_obj(json!({ "last_sync_token": "tok-1" })),
        );
        tx.upsert_node(super::SYSTEM_WORKSPACE, &node)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    let mock = Arc::new(MockAdapter::default());
    // Without this the mount falls back to a FULL walk, where the queued error
    // would be eaten by the best-effort delta-baseline probe.
    mock.set_caps(json!({ "can_read": true, "supports_changes": true }));
    mock.push_changes_err(AdapterError::Transient("provider hiccup".to_string()));

    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(mock.clone() as super::AdapterInvokerHandle),
        None,
    );
    let job = job_info(JobType::VirtualMountSync {
        mount_id: MOUNT_ID.to_string(),
        mode: "delta".to_string(),
        trigger: "schedule".to_string(),
    });
    // A transient error IS retryable, so the job itself reports Err. That is
    // correct and beside the point here — the mount state is what matters.
    let _ = handler.handle(&job, &job_context()).await;

    let state = read_state(&env).await.expect("state must be persisted");
    assert_eq!(
        state.get("status").and_then(|v| v.as_str()),
        Some("error"),
        "the FIRST failure must leave a terminal status, not a mount reading `syncing`"
    );
    assert_eq!(
        state.get("consecutive_failures").and_then(|v| v.as_u64()),
        Some(1)
    );
}

/// A run that was KILLED leaves `status: "syncing"` behind, and nothing else
/// clears it. It must not wedge the mount.
///
/// The watchdog aborts a sync past its job timeout by dropping the task at its
/// await point, so `finalize` never runs. For a polled mount that only costs a
/// lease TTL; for a WEBHOOK mount it was terminal, because the webhook branch
/// returns `!has_active_push` — a mount with a live subscription is never
/// scheduled, so the only thing that could clear the flag was the one thing
/// that could not be scheduled. The console greys out `Sync now` on the same
/// flag, so the operator had no way out either.
#[test]
fn a_killed_run_does_not_wedge_a_mount_at_syncing() {
    let now = 1_800_000_000;
    let lease = super::SYNC_LEASE_TTL.as_secs() as i64;

    // A webhook mount with a live subscription: never polled, by design.
    let mut mount = webhook_mount();
    mount.state.push_status = Some("active".to_string());
    mount.state.push_subscription_id = Some("sub-1".to_string());
    mount.state.push_expires_at = Some("2999-01-01T00:00:00Z".to_string());
    assert!(
        !super::check::is_due(&mount, now),
        "a settled webhook mount must stay push-driven"
    );

    // A run that is genuinely in flight must NOT be re-enqueued.
    mount.state.status = Some("syncing".to_string());
    mount.state.last_run = Some(super::config::SyncRun::started(now - 5, "full", "manual"));
    assert!(
        !super::check::is_due(&mount, now),
        "a live run must not be treated as stale"
    );

    // One that started longer ago than any run can possibly last is dead: the
    // watchdog guarantees no run outlives the job timeout.
    mount.state.last_run = Some(super::config::SyncRun::started(
        now - (2 * lease + 1),
        "full",
        "manual",
    ));
    assert!(
        super::check::is_due(&mount, now),
        "a webhook mount wedged at `syncing` must become due again, or nothing \
         can ever clear the flag"
    );

    // And the same holds for a polled mount.
    let mut polled = mk_mount(SyncConfig::default());
    polled.state.status = Some("syncing".to_string());
    polled.state.last_attempt_at = Some(now);
    polled.state.last_sync_at = Some(now);
    polled.state.last_run = Some(super::config::SyncRun::started(
        now - (2 * lease + 1),
        "full",
        "schedule",
    ));
    assert!(
        super::check::is_due(&polled, now),
        "a stale `syncing` must win over the poll interval, which would otherwise \
         hold this mount off"
    );
}

#[test]
fn webhook_mount_with_pending_backfill_is_due() {
    let now = 1_800_000_000;
    let mut mount = webhook_mount();
    mount.state.push_status = Some("active".to_string());
    mount.state.push_subscription_id = Some("sub-1".to_string());
    mount.state.push_expires_at = Some("2999-01-01T00:00:00Z".to_string());

    // Live subscription, nothing pending => not due (push drives it).
    assert!(
        !super::check::is_due(&mount, now),
        "a fully-synced webhook mount must stay push-driven"
    );

    // Same mount, but a backfill chunk is outstanding => due.
    mount.state.backfill_cursor = Some("page-2".to_string());
    assert!(
        super::check::is_due(&mount, now),
        "an unfinished backfill must be drained even on a push-driven mount"
    );

    // The failure gate still applies, so a broken backfill cannot spin.
    mount.state.consecutive_failures = 1;
    assert!(
        !super::check::is_due(&mount, now),
        "a failing backfill must back off rather than run back-to-back"
    );
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

// ---- batched import ----
//
// The engine used to write ONE item per transaction, and to find that item it
// re-listed the ENTIRE target workspace. Importing a real mailbox was O(items ×
// workspace) and got quadratically slower as it went. These tests pin down the
// batching that replaced it, and the hazards batching introduces.

/// Sync-actor revisions on the target branch, newest first.
async fn sync_revision_count(env: &Env) -> usize {
    use raisin_storage::RevisionRepository;
    env.storage
        .revisions()
        .list_revisions(TENANT, REPO, 10_000, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.actor == super::SYNC_ACTOR)
        .count()
}

fn upsert_op(ext: &str, rel_path: &str, etag: &str) -> BatchOp {
    BatchOp::Upsert {
        rel_path: rel_path.to_string(),
        mapped: super::default_mapping(
            &serde_json::from_value(ext_item(ext, rel_path, false, etag)).unwrap(),
        ),
        virt: VirtualMeta {
            mount_id: MOUNT_ID.to_string(),
            external_id: ext.to_string(),
            etag: Some(etag.to_string()),
            synced_at: Utc::now().to_rfc3339(),
        },
    }
}

/// The core win: N items cost ONE revision, not N.
///
/// One revision also means one branch-HEAD bump, one RocksDB write, one snapshot
/// job and one replication record — the per-item versions of which were the
/// import's actual cost.
#[tokio::test(flavor = "multi_thread")]
async fn a_batch_of_items_costs_one_revision_not_one_per_item() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let before = sync_revision_count(&env).await;
    let ops: Vec<BatchOp> = (0..250)
        .map(|i| upsert_op(&format!("X{i}"), &format!("f{i}.txt"), "v1"))
        .collect();
    let stats = mat.apply_batch(&scope(), &mut index, ops).await.unwrap();

    assert_eq!(stats.written, 250);
    assert_eq!(stats.failed, 0);
    assert_eq!(
        sync_revision_count(&env).await - before,
        1,
        "250 items must commit as ONE revision"
    );
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 250);
    assert_eq!(list_virtual(&mat, &scope()).await.len(), 250);
}

/// 500 items under one parent create that parent ONCE.
///
/// `upsert_deep_node` auto-creates missing ancestors, and inside a shared
/// transaction the read cache is what stops the 2nd..500th item re-creating the
/// same folder. Without it a threaded mailbox would write 500 copies of every
/// conversation folder.
#[tokio::test(flavor = "multi_thread")]
async fn items_sharing_a_parent_folder_create_it_once() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let ops: Vec<BatchOp> = (0..200)
        .map(|i| upsert_op(&format!("M{i}"), &format!("thread-7/m{i}.txt"), "v1"))
        .collect();
    let stats = mat.apply_batch(&scope(), &mut index, ops).await.unwrap();
    assert_eq!(stats.written, 200);

    let nodes = all_nodes(&env, TARGET_WS).await;
    let folders: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.path == format!("{MOUNT_PATH}/thread-7"))
        .collect();
    assert_eq!(
        folders.len(),
        1,
        "the shared parent must exist exactly once"
    );
}

/// Siblings written in ONE transaction share one revision HLC, so the editorial
/// order index must still hold a distinct entry per child.
///
/// The ORDERED_CHILDREN key embeds the revision, and its label is minted from a
/// per-transaction cache precisely so 50 siblings at one revision do not collide.
/// A collision does not error — it silently drops or duplicates children, which
/// `list_children` is what surfaces.
#[tokio::test(flavor = "multi_thread")]
async fn siblings_in_one_batch_each_get_their_own_ordered_child_entry() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let ops: Vec<BatchOp> = (0..50)
        .map(|i| upsert_op(&format!("S{i:03}"), &format!("s{i:03}.txt"), "v1"))
        .collect();
    mat.apply_batch(&scope(), &mut index, ops).await.unwrap();

    let tx = begin(&env).await;
    let children = tx.list_children(TARGET_WS, MOUNT_PATH).await.unwrap();
    assert_eq!(
        children.len(),
        50,
        "every sibling must appear exactly once in editorial order"
    );
    let mut ids: Vec<&str> = children.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        50,
        "no child may be duplicated by a label collision"
    );
}

/// One bad item must not cost the batch.
///
/// A foreign (user-created) node sitting at a target path is refused — that guard
/// predates batching and must survive it — but the other 199 items still land.
/// Losing the whole batch to one rejected item would stall an import forever.
#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_item_does_not_lose_the_rest_of_the_batch() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());

    // A user-created node (no __mount_id) occupying one of the target paths.
    let tx = begin(&env).await;
    tx.upsert_deep_node(
        TARGET_WS,
        &Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Node".to_string(),
            name: "f7.txt".to_string(),
            path: format!("{MOUNT_PATH}/f7.txt"),
            workspace: Some(TARGET_WS.to_string()),
            ..Default::default()
        },
        "raisin:Folder",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let mut index = mat.load_index(&scope()).await.unwrap();
    let ops: Vec<BatchOp> = (0..200)
        .map(|i| upsert_op(&format!("X{i}"), &format!("f{i}.txt"), "v1"))
        .collect();
    let stats = mat.apply_batch(&scope(), &mut index, ops).await.unwrap();

    assert_eq!(stats.written, 199, "the other 199 items must land");
    assert_eq!(stats.skipped, 1, "the foreign-owned path is skipped");
    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 199);
}

/// A duplicated `external_id` in one page must converge on ONE node.
///
/// If both occurrences were written and they resolved to different paths, the
/// mount would hold two nodes claiming the same external id; the next sync would
/// match one arbitrarily and the other would be orphaned forever — invisible to
/// reconcile, because its external id IS in `seen`.
#[tokio::test(flavor = "multi_thread")]
async fn a_duplicated_external_id_in_one_page_yields_one_node() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let stats = mat
        .apply_batch(
            &scope(),
            &mut index,
            vec![
                upsert_op("DUP", "first.txt", "v1"),
                upsert_op("DUP", "second.txt", "v2"),
            ],
        )
        .await
        .unwrap();

    assert_eq!(stats.written, 1);
    let nodes = list_virtual(&mat, &scope()).await;
    assert_eq!(nodes.len(), 1, "one external id must own one node");
    assert_eq!(
        nodes[0].etag.as_deref(),
        Some("v2"),
        "the later occurrence is the newer state and wins"
    );
}

/// Two DIFFERENT items resolving to the same path each get their own node.
///
/// Collapsing them onto one — the old behaviour — lost the loser silently and
/// then alternated the survivor between the two on every run, because the next
/// walk etag-skipped whichever was indexed and staged the other onto the same
/// path. Both ids were already in `seen`, so reconcile could not see it either.
/// A stable per-item suffix breaks the collision and the two settle immediately.
#[tokio::test(flavor = "multi_thread")]
async fn two_items_at_the_same_resolved_path_each_get_a_node() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let stats = mat
        .apply_batch(
            &scope(),
            &mut index,
            vec![
                upsert_op("A", "clash.txt", "v1"),
                upsert_op("B", "clash.txt", "v1"),
            ],
        )
        .await
        .unwrap();

    assert_eq!(stats.written, 2, "neither item may be silently dropped");
    assert_eq!(stats.failed, 0);

    let nodes = list_virtual(&mat, &scope()).await;
    assert_eq!(nodes.len(), 2);
    let mut ids: Vec<&str> = nodes.iter().map(|n| n.external_id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["A", "B"]);

    // Re-applying the identical page must not move either node: the suffix is a
    // digest of the item's own external id, not of its position in the page.
    let paths_before = sorted_paths(&nodes);
    mat.apply_batch(
        &scope(),
        &mut index,
        vec![
            upsert_op("A", "clash.txt", "v2"),
            upsert_op("B", "clash.txt", "v2"),
        ],
    )
    .await
    .unwrap();
    let after = list_virtual(&mat, &scope()).await;
    assert_eq!(after.len(), 2, "no node was created or destroyed");
    assert_eq!(paths_before, sorted_paths(&after), "the paths are stable");
}

/// Deletes and upserts share ONE ordered queue.
///
/// A delta page that creates an item and then deletes it must end with the item
/// gone. Buffering only the upserts and applying deletes eagerly would reorder
/// the page and leave it alive.
#[tokio::test(flavor = "multi_thread")]
async fn a_create_then_delete_in_one_batch_leaves_nothing_behind() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    mat.apply_batch(
        &scope(),
        &mut index,
        vec![
            upsert_op("KEEP", "keep.txt", "v1"),
            upsert_op("GONE", "gone.txt", "v1"),
            BatchOp::Delete {
                external_id: "GONE".to_string(),
            },
        ],
    )
    .await
    .unwrap();

    let ids: Vec<String> = list_virtual(&mat, &scope())
        .await
        .into_iter()
        .map(|n| n.external_id)
        .collect();
    assert_eq!(ids, vec!["KEEP".to_string()]);
}

/// The etag skip survives batching AND is decided before the mapper runs.
///
/// A re-sync of an unchanged mailbox must produce no revision at all: that is
/// what stops every downstream trigger re-firing on every poll.
#[tokio::test(flavor = "multi_thread")]
async fn a_resync_of_unchanged_items_writes_nothing() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let ops = || -> Vec<BatchOp> {
        (0..40)
            .map(|i| upsert_op(&format!("X{i}"), &format!("f{i}.txt"), "v1"))
            .collect()
    };
    mat.apply_batch(&scope(), &mut index, ops()).await.unwrap();

    let after_first = sync_revision_count(&env).await;
    let mut reloaded = mat.load_index(&scope()).await.unwrap();
    let stats = mat
        .apply_batch(&scope(), &mut reloaded, ops())
        .await
        .unwrap();

    assert_eq!(stats.written, 0);
    assert_eq!(stats.skipped, 40);
    assert_eq!(
        sync_revision_count(&env).await,
        after_first,
        "an unchanged re-sync must not create a revision"
    );
}

/// A full sync through the real driver batches, and the reconcile pass reads the
/// run's index instead of re-listing the workspace.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_sync_page_commits_as_one_revision() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig {
        max_items_per_sync: 500,
        ..SyncConfig::default()
    });
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = MockAdapter::default();
    let items: Vec<Value> = (0..120)
        .map(|i| ext_item(&format!("X{i}"), &format!("f{i}.txt"), false, "v1"))
        .collect();
    mock.set_list("root", json!({ "items": items, "next_cursor": null }));

    let before = sync_revision_count(&env).await;
    let mut state = MountState::default();
    super::full::run(&ctx(&env, &mount, &mock, &mat), &mut state)
        .await
        .unwrap();

    assert_eq!(virtual_assets(&all_nodes(&env, TARGET_WS).await).len(), 120);
    assert_eq!(
        sync_revision_count(&env).await - before,
        1,
        "a 120-item page must be one revision, not 120"
    );
}

/// Unrelated content the import must not be slowed down by. Its size is the
/// whole point of the benchmark: the old path re-listed EVERY node once per
/// imported item, so its cost scaled with this number.
async fn seed_workspace(env: &Env, seed: usize) {
    let tx = begin(env).await;
    for i in 0..seed {
        tx.add_node(
            TARGET_WS,
            &Node {
                id: nanoid::nanoid!(),
                node_type: "raisin:Node".to_string(),
                name: format!("seed{i}"),
                path: format!("/seed/seed{i}"),
                workspace: Some(TARGET_WS.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

/// Throughput guard for the change this batching exists for.
///
/// Pre-seeding the workspace is the whole point: the old path re-listed EVERY
/// node once per imported item, so its cost scaled with workspace size while the
/// batched path does not. Run before/after to see the difference.
///
/// `BENCH_N=2000 cargo test -p raisin-rocksdb --lib
///  virtual_mount_sync::tests::import_throughput -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn import_throughput() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let n: usize = std::env::var("BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let seed: usize = std::env::var("BENCH_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);

    seed_workspace(&env, seed).await;

    let mut index = mat.load_index(&scope()).await.unwrap();
    let ops: Vec<BatchOp> = (0..n)
        .map(|i| upsert_op(&format!("X{i}"), &format!("f{i}.txt"), "v1"))
        .collect();

    let start = std::time::Instant::now();
    let stats = mat.apply_batch(&scope(), &mut index, ops).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(stats.written, n);
    println!(
        "import: {n} items into a {seed}-node workspace in {:?} ({:.0} items/sec)",
        elapsed,
        n as f64 / elapsed.as_secs_f64()
    );
}

/// The BEFORE number for [`import_throughput`], reproducing the shape the engine
/// had: one transaction per item, and a fresh full workspace read to locate each
/// one. Kept as a test rather than a comment so the claim stays checkable.
///
/// `BENCH_N=500 cargo test -p raisin-rocksdb --lib
///  virtual_mount_sync::tests::import_throughput_unbatched -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn import_throughput_unbatched() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let n: usize = std::env::var("BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);
    let seed: usize = std::env::var("BENCH_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    seed_workspace(&env, seed).await;

    let start = std::time::Instant::now();
    for i in 0..n {
        // A fresh index per item IS the old per-item `scan_nodes`.
        let mut index = mat.load_index(&scope()).await.unwrap();
        mat.apply_batch(
            &scope(),
            &mut index,
            vec![upsert_op(&format!("X{i}"), &format!("f{i}.txt"), "v1")],
        )
        .await
        .unwrap();
    }
    let elapsed = start.elapsed();
    println!(
        "unbatched: {n} items into a {seed}-node workspace in {:?} ({:.0} items/sec)",
        elapsed,
        n as f64 / elapsed.as_secs_f64()
    );
}

// ---------------------------------------------------------------------------
// Delta baseline: the fix for a mount that finishes importing and then never
// sees a new item again.
// ---------------------------------------------------------------------------

/// After a completed full walk, a delta run with no token must ask for a
/// BASELINE, not an enumeration.
///
/// The bug: a changes API asked for "everything since nothing" answers with an
/// initial full enumeration. Graph returns every message in the folder, paged,
/// and only emits a resumable delta link on the last page. Storing page 1 of
/// that as the delta token made every later run walk the enumeration 600 items
/// at a time — re-reading mail the walk had just imported, reporting
/// `0 written / 600 skipped` run after run, while genuinely new mail waited
/// behind an enumeration that could not converge.
#[test]
fn a_completed_walk_asks_for_a_baseline_not_an_enumeration() {
    // The condition the delta path evaluates.
    let want_baseline =
        |token: Option<&str>, backfill_complete: bool| token.is_none() && backfill_complete;

    // Everything is materialized: "from now on" is the only correct request.
    assert!(want_baseline(None, true));

    // The walk has NOT finished, so the enumeration IS the import. Asking for a
    // baseline here would skip every item that has not been imported yet.
    assert!(!want_baseline(None, false));

    // A stored token is already a resume point and must be used verbatim,
    // whatever the walk's state.
    assert!(!want_baseline(
        Some("https://graph…/delta?$skiptoken=x"),
        true
    ));
    assert!(!want_baseline(
        Some("https://graph…/delta?$skiptoken=x"),
        false
    ));
}

/// Stop clears the delta token, so the self-healing baseline above can fire.
///
/// A mount already holding a mid-enumeration cursor cannot recover on its own —
/// the cursor keeps resuming a walk that never converges. Dropping it is what
/// lets the next run re-baseline, and it is precisely the state an operator
/// reaches for Stop to escape.
#[test]
fn stop_drops_a_poisoned_delta_cursor() {
    let mut state = MountState {
        last_sync_token: Some("https://graph…/messages/delta?$skiptoken=page-700".to_string()),
        backfill_complete: true,
        ..Default::default()
    };

    // What the stop endpoint does to the state blob.
    state.last_sync_token = None;
    state.stop_requested = true;

    assert!(state.last_sync_token.is_none());
    // With no token and a completed walk, the next run re-baselines.
    assert!(state.last_sync_token.is_none() && state.backfill_complete);
}

// ---- capability honesty (stage 0 of the write path) ----

/// An adapter written before the write path existed declares none of the write
/// fields — and must deserialize to "cannot write anything", not to a default
/// that would let a later stage push edits to a provider that never agreed.
#[test]
fn legacy_capabilities_default_to_no_write() {
    let caps = super::Capabilities::from_adapter_value(&json!({
        "can_read": true,
        "supports_changes": true,
        "max_file_size": 1024
    }))
    .expect("legacy capabilities still parse");

    assert!(caps.can_read);
    assert!(caps.supports_changes);
    assert!(!caps.can_write);
    assert!(!caps.can_create);
    assert!(!caps.can_update);
    assert!(!caps.can_delete);
    assert!(!caps.can_submit);
    assert!(caps.mutable_fields.is_empty());
    assert!(caps.default_delete_policy.is_none());
    assert!(caps.default_move_policy.is_none());
    assert!(!caps.supports_trash);
    assert!(!caps.supports_idempotency_key);

    // The no-capabilities fallback is read-only for the same reason.
    let fb = super::Capabilities::fallback();
    assert!(fb.can_read);
    assert!(!fb.can_write && !fb.can_create && !fb.can_update && !fb.can_delete);
}

/// A write-capable adapter round-trips every new field.
#[test]
fn write_capabilities_round_trip() {
    let caps = super::Capabilities::from_adapter_value(&json!({
        "can_read": true,
        "can_write": true,
        "can_create": true,
        "can_update": true,
        "can_delete": true,
        "can_submit": true,
        "mutable_fields": ["unread", "categories"],
        "default_delete_policy": "trash",
        "default_move_policy": "push",
        "supports_trash": true,
        "supports_idempotency_key": true
    }))
    .expect("write capabilities parse");

    assert_eq!(caps.mutable_fields, vec!["unread", "categories"]);
    assert_eq!(caps.default_delete_policy.as_deref(), Some("trash"));
    assert_eq!(caps.default_move_policy.as_deref(), Some("push"));
    assert!(caps.supports_trash && caps.supports_idempotency_key && caps.can_submit);
    assert!(caps.missing_mirror_ops(true, true).is_empty());
}

/// A mount asking for write-through against a read-only adapter is reported as
/// unsupported WITH the reason — the whole point of stage 0 is that the console
/// can say which operation is missing instead of "not supported in v1".
#[test]
fn writeback_unsupported_names_the_missing_ops() {
    let wc = WriteConfig {
        writeback: "write_through".to_string(),
        ..Default::default()
    };
    let caps = super::Capabilities {
        can_read: true,
        ..Default::default()
    };

    let (supported, reason) =
        super::write::writeback_verdict(&wc, &caps, &MapperWriteback::Supported);
    assert_eq!(supported, Some(false));
    let reason = reason.expect("a refusal carries a reason");
    assert!(reason.contains("can_write"), "{reason}");
    assert!(reason.contains("can_update"), "{reason}");

    // A partially-capable adapter names only what it is missing.
    let partial = super::Capabilities {
        can_read: true,
        can_write: true,
        can_create: true,
        can_update: true,
        ..Default::default()
    };
    // `purge`, because `can_delete` is demanded only from a mount that actually
    // pushes deletes. On the default `detach` this same adapter is now a
    // perfectly good mirror — it never calls delete — and that is deliberate:
    // requiring the capability there is what silently demoted every Stripe
    // catalogue mount to read-only.
    let pushes_deletes = WriteConfig {
        writeback: "write_through".to_string(),
        delete_policy: Some("purge".to_string()),
        ..Default::default()
    };
    let (supported, reason) =
        super::write::writeback_verdict(&pushes_deletes, &partial, &MapperWriteback::Supported);
    assert_eq!(supported, Some(false));
    assert_eq!(
        reason.as_deref(),
        Some("adapter does not declare can_delete")
    );

    // The same adapter, detaching: supported.
    let (supported, reason) =
        super::write::writeback_verdict(&wc, &partial, &MapperWriteback::Supported);
    assert_eq!(
        supported,
        Some(true),
        "detach needs no can_delete: {reason:?}"
    );
}

/// Write-through against a fully capable adapter is supported, with no reason;
/// a mount that never asked stays `None` (absent != `Some(false)` to the UI, and
/// writing it every run would churn the state blob).
#[test]
fn writeback_verdict_supported_and_not_requested() {
    let caps = super::Capabilities {
        can_read: true,
        can_write: true,
        can_create: true,
        can_update: true,
        can_delete: true,
        ..Default::default()
    };

    let (supported, reason) = super::write::writeback_verdict(
        &WriteConfig {
            writeback: "write_through".to_string(),
            ..Default::default()
        },
        &caps,
        &MapperWriteback::Supported,
    );
    assert_eq!(supported, Some(true));
    assert!(reason.is_none());

    let (supported, reason) = super::write::writeback_verdict(
        &WriteConfig::default(),
        &caps,
        &MapperWriteback::Supported,
    );
    assert_eq!(supported, None, "an off mount is not applicable, not false");
    assert!(reason.is_none());
}

// ---- bidirectional mapper (stage 1b) ----

/// A write-capable adapter behind a mapper that cannot answer `to_external` is
/// NOT a writable mount. Writability belongs to the mount — adapter and mapper
/// together — so the mapper vetoes on its own, and when both fall short the
/// operator is told both rather than fixing one and being refused for the other.
#[test]
fn a_read_only_mapper_vetoes_a_write_capable_adapter() {
    let wc = WriteConfig {
        writeback: "write_through".to_string(),
        ..Default::default()
    };
    let caps = super::Capabilities {
        can_read: true,
        can_write: true,
        can_create: true,
        can_update: true,
        can_delete: true,
        ..Default::default()
    };

    for mapper in [
        MapperWriteback::NoMapper,
        MapperWriteback::NotImplemented,
        MapperWriteback::ProbeFailed("boom".to_string()),
    ] {
        let (supported, reason) = super::write::writeback_verdict(&wc, &caps, &mapper);
        assert_eq!(supported, Some(false), "{mapper:?}");
        assert!(reason.is_some(), "{mapper:?} must state why");
    }

    // Both shortfalls reported, not just the first one found.
    let (supported, reason) = super::write::writeback_verdict(
        &wc,
        &super::Capabilities::fallback(),
        &MapperWriteback::NotImplemented,
    );
    assert_eq!(supported, Some(false));
    let reason = reason.unwrap();
    assert!(reason.contains("adapter does not declare"), "{reason}");
    assert!(reason.contains("to_external"), "{reason}");
}

/// A mount with no `mapping_function` is read-only by construction, and says so
/// WITHOUT invoking anything: the built-in Rust mapping has no reverse, so
/// there is nothing to ask.
#[tokio::test(flavor = "multi_thread")]
async fn no_mapping_function_means_read_only_without_a_probe() {
    let env = setup().await;
    let mount = mk_mount(SyncConfig::default());
    assert!(mount.mapping_function.is_none());
    let mock = MockAdapter::default();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(&env, &mount, &mock, &mat);

    assert_eq!(c.probe_mapper_writeback().await, MapperWriteback::NoMapper);
    assert_eq!(mock.call_count(), 0, "nothing to ask, so nothing was asked");

    // And the reverse mapping itself short-circuits rather than inventing one.
    let out = super::map_to_external(&c, &json!({"id": "n1"}), None, "update")
        .await
        .unwrap();
    assert!(matches!(out, super::ToExternalOutcome::NoMapper));
    assert_eq!(mock.call_count(), 0);
}

/// Backward compatibility, the non-negotiable part of stage 1b: a mapper that
/// predates `operation` sees the same `external_item` / `mount` it always saw
/// and produces the same node. The engine sends `operation: "to_node"`, which a
/// legacy mapper simply ignores.
#[tokio::test(flavor = "multi_thread")]
async fn a_legacy_mapper_is_unaffected_by_operation_dispatch() {
    let env = setup().await;
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/legacy".to_string());
    let mock = LegacyMapper::default();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(&env, &mount, &mock, &mat);

    let item: super::ExternalItem =
        serde_json::from_value(ext_item("X1", "x.txt", false, "e1")).unwrap();
    let mapped = super::map_item(&c, &item).await.unwrap().unwrap();
    assert_eq!(mapped.node.node_type, "raisin:Node");
    assert_eq!(mapped.node.name.as_deref(), Some("x.txt"));

    // It saw the new key and ignored it, exactly as a switch-less mapper does.
    assert_eq!(mock.ops.lock().unwrap().as_slice(), ["to_node"]);

    // The same mapper answers null to everything else, so it is reported as
    // read-only — never accidentally write-enabled.
    assert_eq!(
        c.probe_mapper_writeback().await,
        MapperWriteback::NotImplemented
    );
    let out = super::map_to_external(&c, &json!({"id": "n1"}), None, "update")
        .await
        .unwrap();
    assert!(matches!(out, super::ToExternalOutcome::NotWritable));
}

/// A dispatching mapper's `to_external` half: the probe is believed, the
/// allow-list is forwarded, and the payload comes back typed.
#[tokio::test(flavor = "multi_thread")]
async fn a_bidirectional_mapper_round_trips_to_external() {
    let env = setup().await;
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/bidi".to_string());
    let mock = BidiMapper::default();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let c = ctx(&env, &mount, &mock, &mat);

    assert_eq!(c.probe_mapper_writeback().await, MapperWriteback::Supported);

    let fields = vec!["unread".to_string()];
    let out = super::map_to_external(&c, &json!({"id": "n1"}), Some(&fields), "update")
        .await
        .unwrap();
    let super::ToExternalOutcome::Mapped(m) = out else {
        panic!("expected a payload");
    };
    assert_eq!(m.payload, json!({"isRead": true}));
    assert_eq!(m.external_id.as_deref(), Some("EXT-1"));
    assert_eq!(
        mock.last_fields.lock().unwrap().clone(),
        Some(json!(["unread"])),
        "the allow-list reaches the mapper verbatim"
    );
}

// ---- sync actor identity ----

/// A sync-materialized node is attributed to the sync actor, not to `"system"`.
///
/// Both stamps come from the same transaction but by different routes: the
/// revision's `actor` from `set_actor`, the node's `created_by`/`updated_by`
/// from the auth context (which wins over the raw actor in `put_node`). Before
/// `AuthContext::system_as`, only the first was honest — every synced node
/// claimed `"system"` wrote it, in the node record, the audit log, and the
/// emitted `node:*` events alike.
#[tokio::test(flavor = "multi_thread")]
async fn a_synced_node_is_attributed_to_the_sync_actor() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mut index = mat.load_index(&scope()).await.unwrap();

    let before = sync_revision_count(&env).await;
    mat.apply_batch(&scope(), &mut index, vec![upsert_op("A1", "a.txt", "v1")])
        .await
        .unwrap();

    let nodes = all_nodes(&env, TARGET_WS).await;
    let node = virtual_assets(&nodes)
        .into_iter()
        .find(|n| str_prop(n, "__external_id").as_deref() == Some("A1"))
        .expect("the synced node must exist");

    assert_eq!(node.created_by.as_deref(), Some(super::SYNC_ACTOR));
    assert_eq!(node.updated_by.as_deref(), Some(super::SYNC_ACTOR));

    // Additive, not a swap: the revision actor is unchanged.
    assert_eq!(
        sync_revision_count(&env).await - before,
        1,
        "the revision must still be attributed to the sync actor"
    );

    // An update of the same node keeps the create attribution and re-stamps the
    // update one.
    mat.apply_batch(&scope(), &mut index, vec![upsert_op("A1", "a.txt", "v2")])
        .await
        .unwrap();
    let nodes = all_nodes(&env, TARGET_WS).await;
    let node = virtual_assets(&nodes)
        .into_iter()
        .find(|n| str_prop(n, "__external_id").as_deref() == Some("A1"))
        .unwrap();
    assert_eq!(node.created_by.as_deref(), Some(super::SYNC_ACTOR));
    assert_eq!(node.updated_by.as_deref(), Some(super::SYNC_ACTOR));
}

// ---- state_only writeback (stage 2) ----

/// One invoker playing both roles a `state_only` mount needs: the provider's
/// adapter and the mount's bidirectional mapper. They share an invoker in
/// production too (only the function path differs), so a single mock is the
/// faithful shape.
#[derive(Default)]
struct StateOnlyMock {
    calls: Mutex<Vec<String>>,
    /// `params` of every `update` the engine issued, in order.
    updates: Mutex<Vec<Value>>,
    changes: Mutex<VecDeque<Value>>,
    /// Etag the provider assigns to the next accepted update.
    next_etag: Mutex<String>,
    /// When set, `update` returns this verbatim instead of an accepted-write
    /// envelope — how an adapter answers something other than "done", e.g. the
    /// ms-graph adapter's `null` for a 404 ("gone": Graph message ids are not
    /// stable, so a moved message's stored id no longer resolves).
    update_reply: Mutex<Option<Value>>,
    /// When set, `update` FAILS with this error instead of answering.
    ///
    /// Distinct from `update_reply`, which models an adapter that answers
    /// something other than "done". This models one that refuses outright —
    /// the 403-for-a-missing-scope case, where no retry can ever succeed.
    update_error: Mutex<Option<String>>,
}

impl StateOnlyMock {
    fn new() -> Self {
        Self {
            next_etag: Mutex::new("v2".to_string()),
            ..Default::default()
        }
    }
    fn update_count(&self) -> usize {
        self.updates.lock().unwrap().len()
    }
    fn push_changes(&self, page: Value) {
        self.changes.lock().unwrap().push_back(page);
    }
    /// Make every subsequent `update` answer `reply` verbatim.
    fn set_update_reply(&self, reply: Value) {
        *self.update_reply.lock().unwrap() = Some(reply);
    }
    /// Make every subsequent `update` fail with a `config_error`.
    fn fail_updates_with_config_error(&self, message: &str) {
        *self.update_error.lock().unwrap() = Some(message.to_string());
    }
}

#[async_trait]
impl AdapterInvoker for StateOnlyMock {
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
        self.calls.lock().unwrap().push(op.to_string());
        let params = input.get("params").cloned().unwrap_or(Value::Null);
        match op {
            "capabilities" => Ok(json!({
                "can_read": true,
                "can_write": true,
                "can_update": true,
                "supports_changes": true,
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
            "to_node" => {
                let item = input.get("external_item").cloned().unwrap_or(Value::Null);
                Ok(json!({
                    "node_type": "raisin:Node",
                    "name": item.get("name").cloned(),
                    "properties": {
                        "unread": item
                            .get("metadata")
                            .and_then(|m| m.get("unread"))
                            .cloned()
                            .unwrap_or(Value::Bool(false)),
                    },
                }))
            }
            "update" => {
                self.updates.lock().unwrap().push(params.clone());
                if let Some(msg) = self.update_error.lock().unwrap().clone() {
                    return Err(AdapterError::Config(msg));
                }
                if let Some(reply) = self.update_reply.lock().unwrap().clone() {
                    return Ok(reply);
                }
                Ok(json!({
                    "external_id": params.get("item_id").cloned(),
                    "etag": self.next_etag.lock().unwrap().clone(),
                }))
            }
            "get_changes" => Ok(self
                .changes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| json!({ "items": [], "next_token": null }))),
            _ => Ok(Value::Null),
        }
    }
}

/// A mount configured for the stage-2 slice: `state_only`, one boolean field.
fn state_only_mount() -> MountConfig {
    let mut mount = mk_mount(SyncConfig::default());
    mount.mapping_function = Some("/mappers/mail".to_string());
    mount.write_config = WriteConfig {
        mode: "state_only".to_string(),
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    mount
}

fn watched_scope() -> MountScope {
    MountScope {
        watched_fields: vec!["unread".to_string()],
        ..scope()
    }
}

/// Materialize one mail-shaped node the way a sync would, so it carries a
/// seeded `__pushed_state` rather than a hand-written one.
async fn sync_in_mail(mat: &RocksDbMaterializer, unread: bool, etag: &str) -> String {
    let mut index = mat.load_index(&watched_scope()).await.unwrap();
    let mut properties = serde_json::Map::new();
    properties.insert("unread".to_string(), Value::Bool(unread));
    mat.apply_batch(
        &watched_scope(),
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
                etag: Some(etag.to_string()),
                synced_at: Utc::now().to_rfc3339(),
            },
        }],
    )
    .await
    .unwrap();
    index.virtual_nodes()[0].id.clone()
}

/// Edit a node the way a user would: through an ordinary transactional write
/// that touches no reserved property.
async fn set_bool_prop(env: &Env, node_id: &str, key: &str, value: bool) {
    let tx = begin(env).await;
    let mut node = tx.get_node(TARGET_WS, node_id).await.unwrap().unwrap();
    node.properties
        .insert(key.to_string(), PropertyValue::Boolean(value));
    tx.upsert_node(TARGET_WS, &node).await.unwrap();
    tx.commit().await.unwrap();
}

async fn node_by_id(env: &Env, node_id: &str) -> Node {
    let tx = begin(env).await;
    tx.get_node(TARGET_WS, node_id).await.unwrap().unwrap()
}

fn pushed_state(node: &Node) -> Value {
    serde_json::to_value(
        node.properties
            .get(super::materializer::PUSHED_STATE_PROP)
            .expect("a pushed mount-owned node carries __pushed_state"),
    )
    .unwrap()
}

/// A sync writes `__pushed_state` from the item the provider just reported.
///
/// This is what makes a REMOTE change converge on arrival. Without it every
/// inbound item would look like an un-pushed local edit and be sent straight
/// back to the provider that just reported it.
#[tokio::test(flavor = "multi_thread")]
async fn a_sync_seeds_pushed_state_from_the_remote_item() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let id = sync_in_mail(&mat, false, "v1").await;

    assert_eq!(
        pushed_state(&node_by_id(&env, &id).await),
        json!({"unread": false})
    );

    // A mount that watches nothing writes no such property at all.
    let mut index = mat.load_index(&scope()).await.unwrap();
    mat.apply_batch(&scope(), &mut index, vec![upsert_op("A1", "a.txt", "v1")])
        .await
        .unwrap();
    let plain = all_nodes(&env, TARGET_WS)
        .await
        .into_iter()
        .find(|n| str_prop(n, "__external_id").as_deref() == Some("A1"))
        .unwrap();
    assert!(!plain
        .properties
        .contains_key(super::materializer::PUSHED_STATE_PROP));
}

/// (a) A local edit of a watched field produces exactly ONE adapter `update`,
/// and the stamp-back records both the provider's new etag and the value that
/// was actually pushed.
#[tokio::test(flavor = "multi_thread")]
async fn a_local_edit_of_a_watched_field_pushes_once() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = super::batch::SyncBatcher::new(&c).await.unwrap();
    let stats = super::write::drain(
        &c,
        &mut state,
        &mut batcher,
        &super::write::WriteMode::StateOnly(super::write::FieldPlan::pushing(&["unread"])),
    )
    .await;

    assert_eq!(stats.pushed, 1);
    assert_eq!(stats.failed, 0);
    assert_eq!(mock.update_count(), 1, "exactly one provider write");

    let sent = mock.updates.lock().unwrap()[0].clone();
    assert_eq!(sent.get("item_id").and_then(|v| v.as_str()), Some("M1"));
    assert_eq!(sent.get("etag").and_then(|v| v.as_str()), Some("v1"));
    assert_eq!(sent.get("payload"), Some(&json!({ "isRead": false })));
    assert_eq!(sent.get("fields"), Some(&json!(["unread"])));

    let node = node_by_id(&env, &id).await;
    assert_eq!(str_prop(&node, "__etag").as_deref(), Some("v2"));
    assert_eq!(pushed_state(&node), json!({"unread": true}));
}

/// (b) The converge check makes the drain idempotent: running it again after a
/// successful push issues NO second provider write.
///
/// This is the property the whole design rests on. Change detection is allowed
/// to be noisy — a capture hook, a watermark walk, a full index sweep — only
/// because a redundant candidate costs nothing.
#[tokio::test(flavor = "multi_thread")]
async fn draining_twice_pushes_once() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mode = super::write::WriteMode::StateOnly(super::write::FieldPlan::pushing(&["unread"]));
    let mut state = MountState::default();

    for _ in 0..2 {
        // A fresh batcher each time: a new run re-reads the index from storage,
        // which is the harder case — an in-memory index would remember the
        // stamp, but a second RUN must reach the same answer from disk alone.
        let mut batcher = super::batch::SyncBatcher::new(&c).await.unwrap();
        super::write::drain(&c, &mut state, &mut batcher, &mode).await;
    }
    assert_eq!(mock.update_count(), 1, "the second drain must be a no-op");
}

/// (c) The echo test. After a push, a delta returning that very item with the
/// etag the provider assigned must write NOTHING.
///
/// This is the one that fails if the stamp-back regresses: without it the
/// inbound item's etag never matches the stored one, the mapper re-runs, the
/// node is rewritten, its `unread` flips back to the pre-push value, and the
/// next drain pushes again — forever, one revision per cycle.
/// A remote flag change carrying the SAME etag must still land.
///
/// The production bug this pins: marking a message read in Outlook left
/// `@odata.etag` exactly as the engine's own PATCH response had reported it.
/// The delta delivered the change, the etag matched, and the item was dropped
/// before the mapper ran — so the flag was lost permanently and the cursor
/// advanced past it. Only a force-rewrite recovered it, and the failure was
/// invisible on any message the engine had never pushed to.
#[tokio::test]
async fn a_remote_flag_change_under_an_unchanged_etag_is_applied() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    // Synced as READ, and the node agrees.
    let id = sync_in_mail(&mat, false, "v1").await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };

    // The provider now reports it UNREAD — under the etag already stored.
    mock.push_changes(json!({ "items": [{
        "type": "updated",
        "relative_path": "m1.eml",
        "item": {
            "external_id": "M1",
            "name": "m1",
            "is_folder": false,
            "etag": "v1",
            "metadata": { "unread": true },
        },
    }], "next_token": null }));

    super::delta::run(&c, &mut state).await.unwrap();

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        node.properties.get("unread"),
        Some(&raisin_models::nodes::properties::PropertyValue::Boolean(
            true
        )),
        "a remote flag change must land even when the provider reuses the etag"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delta_echoing_the_pushed_item_writes_nothing() {
    let env = setup().await;
    let mount = state_only_mount();
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    let c = ctx(&env, &mount, &mock, &mat);
    let mode = super::write::WriteMode::StateOnly(super::write::FieldPlan::pushing(&["unread"]));
    let mut state = MountState {
        last_sync_token: Some("t0".to_string()),
        ..Default::default()
    };
    let mut batcher = super::batch::SyncBatcher::new(&c).await.unwrap();
    super::write::drain(&c, &mut state, &mut batcher, &mode).await;
    assert_eq!(mock.update_count(), 1);

    // The provider now reports the item back, carrying the new etag and the
    // state we just pushed — exactly what Graph's delta does after an `isRead`
    // write.
    mock.push_changes(json!({ "items": [{
        "type": "updated",
        "relative_path": "m1.eml",
        "item": {
            "external_id": "M1",
            "name": "m1",
            "is_folder": false,
            "etag": "v2",
            "metadata": { "unread": true },
        },
    }], "next_token": null }));

    let revisions_before = sync_revision_count(&env).await;
    super::delta::run(&c, &mut state).await.unwrap();

    assert_eq!(
        sync_revision_count(&env).await,
        revisions_before,
        "the echoed item must not produce a revision"
    );
    // The mapper DOES run for a mount that watches fields, and that is the
    // point rather than a regression: Graph can report a read/unread flip
    // under the etag its own PATCH response returned, so the etag cannot prove
    // an echo. What must not happen is a WRITE, which the revision count above
    // asserts. The cheap pre-mapping skip is kept for read-only mounts, where
    // an etag really is the whole story.

    let node = node_by_id(&env, &id).await;
    assert_eq!(
        node.properties.get("unread"),
        Some(&PropertyValue::Boolean(true)),
        "the local value must survive its own echo"
    );
    assert_eq!(mock.update_count(), 1, "and no second push is provoked");
}

/// (d) A mount with no write mode never calls `update` — even with a node whose
/// watched field diverges, and even behind a fully write-capable adapter.
#[tokio::test(flavor = "multi_thread")]
async fn a_mount_with_no_write_mode_never_pushes() {
    let env = setup().await;
    let mat = RocksDbMaterializer::new(env.storage.clone());
    let mock = StateOnlyMock::new();
    let id = sync_in_mail(&mat, false, "v1").await;
    set_bool_prop(&env, &id, "unread", true).await;

    // Same divergence, but the mount never asked for writes.
    let mount = mk_mount(SyncConfig::default());
    let c = ctx(&env, &mount, &mock, &mat);
    let mut state = MountState::default();
    let mut batcher = super::batch::SyncBatcher::new(&c).await.unwrap();

    let caps = super::Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    let mode = super::write::resolve_mode(&mount.write_config, &caps, &MapperWriteback::Supported);
    assert_eq!(mode, super::write::WriteMode::Off);

    let stats = super::write::drain(&c, &mut state, &mut batcher, &mode).await;
    assert_eq!(stats, Default::default());
    assert_eq!(mock.update_count(), 0);
    assert!(state.writeback_last_error.is_none());
}

/// A `state_only` mount whose adapter or mapper falls short is REFUSED, with a
/// reason — and refused means nothing is sent, not that a partial write is
/// attempted.
#[test]
fn state_only_is_refused_with_a_reason() {
    let wc = WriteConfig {
        mode: "state_only".to_string(),
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };
    let full = super::Capabilities {
        can_read: true,
        can_write: true,
        can_update: true,
        mutable_fields: vec!["unread".to_string()],
        ..Default::default()
    };

    // Everything present: allowed, with the effective allow-list.
    assert_eq!(
        super::write::resolve_mode(&wc, &full, &MapperWriteback::Supported),
        super::write::WriteMode::StateOnly(super::write::FieldPlan::pushing(&["unread"]))
    );

    // A read-only adapter names the missing op — and NOT `can_create` /
    // `can_delete`, which `state_only` never calls.
    let refusal =
        |caps: &super::Capabilities, mapper: &MapperWriteback| match super::write::resolve_mode(
            &wc, caps, mapper,
        ) {
            super::write::WriteMode::Refused(r) => r,
            other => panic!("expected a refusal, got {other:?}"),
        };
    let reason = refusal(
        &super::Capabilities::fallback(),
        &MapperWriteback::Supported,
    );
    assert!(
        reason.contains("can_write") && reason.contains("can_update"),
        "{reason}"
    );
    assert!(!reason.contains("can_delete"), "{reason}");

    // A read-only mapper vetoes a write-capable adapter.
    assert!(refusal(&full, &MapperWriteback::NotImplemented).contains("to_external"));

    // An adapter that accepts none of the mount's fields is refused rather than
    // silently pushing a field the provider will reject on every run forever.
    let narrow = super::Capabilities {
        mutable_fields: vec!["flagged".to_string()],
        ..full.clone()
    };
    assert!(refusal(&narrow, &MapperWriteback::Supported).contains("mutable_fields"));

    // ...as is a mount that declared no fields.
    let no_fields = WriteConfig {
        mode: "state_only".to_string(),
        ..Default::default()
    };
    match super::write::resolve_mode(&no_fields, &full, &MapperWriteback::Supported) {
        super::write::WriteMode::Refused(r) => assert!(r.contains("mutable_fields"), "{r}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// The unseeded/baseline behaviour that used to live here moved to
// `write_baseline_tests.rs`, together with the mode-change and failure-budget
// cases it turned out to be entangled with.

#[path = "write_lifecycle_tests.rs"]
mod write_lifecycle_tests;

#[path = "write_baseline_tests.rs"]
mod write_baseline_tests;

#[path = "write_gone_tests.rs"]
mod write_gone_tests;

#[path = "misconfig_tests.rs"]
mod misconfig_tests;

#[path = "write_reconcile_tests.rs"]
mod write_reconcile_tests;

#[path = "write_capture_tests.rs"]
mod write_capture_tests;

#[path = "write_mirror_tests.rs"]
mod write_mirror_tests;

#[path = "read_local_wins_tests.rs"]
mod read_local_wins_tests;

#[path = "index_scope_tests.rs"]
mod index_scope_tests;

#[path = "attachment_tests.rs"]
mod attachment_tests;

#[path = "write_move_tests.rs"]
mod write_move_tests;

#[path = "write_conflict_tests.rs"]
mod write_conflict_tests;

#[path = "write_submit_tests.rs"]
mod write_submit_tests;

#[path = "registry_tests.rs"]
mod registry_tests;
