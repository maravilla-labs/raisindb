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
        public_origin: None,
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
    let mut index = mat.load_index(&scope()).await.unwrap();
    let wrote = upsert_one(&mat, &scope(), &mut index, "a", mapped.clone(), virt.clone()).await;
    assert!(wrote, "first upsert writes");
    let again = upsert_one(&mat, &scope(), &mut index, "a", mapped.clone(), virt.clone()).await;
    assert!(!again, "same etag must skip the write");

    // The skip must also hold for an index freshly re-read from storage, not
    // just for the in-memory one this run mutated.
    let mut reloaded = mat.load_index(&scope()).await.unwrap();
    let third = upsert_one(&mat, &scope(), &mut reloaded, "a", mapped, virt).await;
    assert!(!third, "same etag must still skip after reloading the index");
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
    assert_eq!(folders.len(), 1, "the shared parent must exist exactly once");
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
    assert_eq!(ids.len(), 50, "no child may be duplicated by a label collision");
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

/// Two DIFFERENT items resolving to the same path collapse to one node, and only
/// one of them may claim it — otherwise both would be re-imported every sync,
/// each overwriting the other's `__external_id` forever.
#[tokio::test(flavor = "multi_thread")]
async fn two_items_at_the_same_resolved_path_keep_a_single_owner() {
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

    assert_eq!(stats.written, 1);
    let nodes = list_virtual(&mat, &scope()).await;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].external_id, "B", "the last item wins the path");
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
    let stats = mat.apply_batch(&scope(), &mut reloaded, ops()).await.unwrap();

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
