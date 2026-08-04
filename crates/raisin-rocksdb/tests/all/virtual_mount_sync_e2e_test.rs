//! End-to-end virtual-mount syncs: the job handler, real config nodes, a real
//! RocksDB store, and a provider mock that behaves like a paging remote.
//!
//! The in-crate unit tests (`virtual_mount_sync/tests.rs`) cover the decision
//! functions and the materializer in isolation. Nothing drove a whole run
//! through `VirtualMountSyncHandler::handle` against a provider with more items
//! than one page, and two production failures went out through that gap:
//!
//!  1. Every materialized node was written with an EMPTY id, because
//!     `NodeService::create` does not mint one. The first write took, and every
//!     later write collided with it. No test wrote a node through the handler,
//!     so no test could see it.
//!  2. The delta baseline captured page 1 of a full re-enumeration instead of a
//!     resumable "from now on" token, so a mount that had finished importing
//!     never received a new item again — it re-read the same mailbox page every
//!     run, forever. No test ran a sync to completion and then asked whether a
//!     NEW item arrives.
//!
//! Everything here therefore asserts on observable outcomes — nodes that exist,
//! the run counters the handler returns, `check::is_due` — rather than on the
//! sequence of adapter calls, so a refactor of the internals does not rewrite
//! the tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::{
    persist_mount_state, read_mount_state, virtual_mount_check as check, AdapterError,
    AdapterInvoker, AdapterInvokerHandle, MountConfig, MountScope, MountState, RocksDBStorage,
    VirtualMountSyncHandler, SYSTEM_WORKSPACE,
};
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, RepoScope, RepositoryManagementRepository, Storage, WorkspaceRepository,
};
use serde_json::{json, Value};

const TENANT: &str = "default";
const REPO: &str = "vm-e2e";
const BRANCH: &str = "main";
const TARGET_WS: &str = "default";
const MOUNT_PATH: &str = "/drive";
const MOUNT_ID: &str = "mount-e2e";

// ---------------------------------------------------------------------------
// environment
// ---------------------------------------------------------------------------

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
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(storage.clone(), TENANT, REPO, BRANCH)
        .await
        .unwrap();
    for ws in raisin_core::workspace_init::load_global_workspaces() {
        storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), ws)
            .await
            .unwrap();
    }
    Env { _dir: dir, storage }
}

// ---------------------------------------------------------------------------
// provider mock
// ---------------------------------------------------------------------------

/// A remote that pages, keeps a change log, and — crucially — distinguishes a
/// resumable BASELINE token from an ENUMERATION cursor, exactly as Microsoft
/// Graph does. That distinction is the whole point of test 2: a baseline says
/// "changes from now on"; an enumeration cursor replays the existing corpus a
/// page at a time and only reaches genuinely new items once it has drained.
struct Provider {
    items: Mutex<Vec<Value>>,
    /// Appended change entries; a delta token is an index into this log.
    changes: Mutex<Vec<Value>>,
    list_page: usize,
    enum_page: usize,
}

impl Provider {
    fn new(list_page: usize, enum_page: usize) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            changes: Mutex::new(Vec::new()),
            list_page,
            enum_page,
        }
    }

    /// Seed `n` files, `file-000`…, all with etag `v1`.
    fn seed(&self, n: usize) {
        let mut items = self.items.lock().unwrap();
        for i in 0..n {
            items.push(ext_item(
                &format!("F{i:04}"),
                &format!("file-{i:04}.txt"),
                "v1",
            ));
        }
    }

    fn item_count(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    /// Add one item upstream AND record it in the change log, the way a real
    /// provider does: it is visible both to a full listing and to a delta feed
    /// taken from a baseline captured before it existed.
    fn add_item(&self, id: &str, name: &str) {
        let item = ext_item(id, name, "v1");
        self.items.lock().unwrap().push(item.clone());
        self.changes.lock().unwrap().push(json!({
            "type": "created", "item": item, "relative_path": name,
        }));
    }

    fn list(&self, cursor: &str) -> Value {
        let items = self.items.lock().unwrap();
        let off: usize = cursor
            .strip_prefix("off:")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let end = (off + self.list_page).min(items.len());
        let page: Vec<Value> = items[off.min(items.len())..end].to_vec();
        let next = if end < items.len() {
            Value::String(format!("off:{end}"))
        } else {
            Value::Null
        };
        json!({ "items": page, "next_cursor": next, "total": items.len() })
    }

    fn get_changes(&self, since: Option<&str>, baseline_only: bool) -> Value {
        // "From now on": no items, and a cursor positioned at the end of the
        // change log. This is what a correct full walk asks for once it has
        // materialized everything.
        if baseline_only {
            let n = self.changes.lock().unwrap().len();
            return json!({ "items": [], "next_token": format!("chg:{n}") });
        }
        match since {
            // An incremental request from a real baseline.
            Some(t) if t.starts_with("chg:") => {
                let from: usize = t[4..].parse().unwrap_or(0);
                let changes = self.changes.lock().unwrap();
                let page: Vec<Value> = changes[from.min(changes.len())..].to_vec();
                json!({ "items": page, "next_token": format!("chg:{}", changes.len()) })
            }
            // An enumeration in progress. The snapshot size is carried in the
            // token so a walk that started before an item was added cannot see
            // it — which is precisely why storing one of these as the delta
            // baseline stalls a mount.
            Some(t) if t.starts_with("enum:") => {
                let mut parts = t[5..].split(':');
                let off: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let snap: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                self.enumerate(off, snap)
            }
            // "Everything since nothing" — answered with a full enumeration,
            // NOT a baseline. Graph does exactly this.
            _ => {
                let snap = self.items.lock().unwrap().len();
                self.enumerate(0, snap)
            }
        }
    }

    fn enumerate(&self, off: usize, snap: usize) -> Value {
        if off >= snap {
            // Enumeration drained; only now does the provider hand over a
            // resumable delta cursor.
            let n = self.changes.lock().unwrap().len();
            return json!({ "items": [], "next_token": format!("chg:{n}") });
        }
        let items = self.items.lock().unwrap();
        let end = (off + self.enum_page).min(snap).min(items.len());
        let page: Vec<Value> = items[off..end]
            .iter()
            .map(|i| {
                json!({
                    "type": "created",
                    "item": i.clone(),
                    "relative_path": i["name"].as_str().unwrap_or_default(),
                })
            })
            .collect();
        json!({ "items": page, "next_token": format!("enum:{end}:{snap}") })
    }
}

#[async_trait]
impl AdapterInvoker for Provider {
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
        let params = input.get("params").cloned().unwrap_or(Value::Null);
        Ok(match op {
            "capabilities" => json!({
                "can_read": true, "supports_changes": true,
                "supports_webhooks": false, "supports_push": false,
            }),
            "list" => self.list(params.get("cursor").and_then(|v| v.as_str()).unwrap_or("")),
            "get_changes" => self.get_changes(
                params.get("since_token").and_then(|v| v.as_str()),
                params
                    .get("baseline_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            ),
            _ => Value::Null,
        })
    }
}

fn ext_item(id: &str, name: &str, etag: &str) -> Value {
    json!({
        "external_id": id, "name": name, "is_folder": false,
        "etag": etag, "mime_type": "text/plain", "size_bytes": 10,
    })
}

// ---------------------------------------------------------------------------
// driving the handler
// ---------------------------------------------------------------------------

/// One sync run through the real job handler. Returns the run summary the
/// handler publishes as the job result (`written` / `skipped` / `deleted` /
/// `outcome`), which is what the console renders.
async fn sync(env: &Env, provider: &Arc<Provider>, mode: &str) -> Value {
    let handler = VirtualMountSyncHandler::new(
        env.storage.clone(),
        Some(provider.clone() as AdapterInvokerHandle),
        None,
    );
    let job = JobInfo {
        id: JobId("test-job".to_string()),
        job_type: JobType::VirtualMountSync {
            mount_id: MOUNT_ID.to_string(),
            mode: mode.to_string(),
            trigger: "manual".to_string(),
        },
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
    };
    let context = JobContext {
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        workspace_id: SYSTEM_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::now(),
        metadata: HashMap::new(),
    };
    handler
        .handle(&job, &context)
        .await
        .expect("sync job failed")
        .unwrap_or(Value::Null)
}

fn count(summary: &Value, key: &str) -> u64 {
    summary.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// storage helpers
// ---------------------------------------------------------------------------

async fn begin(env: &Env) -> Box<dyn raisin_storage::transactional::TransactionalContext> {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
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

/// Every node this mount owns, read back from storage.
async fn mount_nodes(env: &Env) -> Vec<Node> {
    let tx = begin(env).await;
    tx.scan_nodes(TARGET_WS)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| str_prop(n, "__mount_id").as_deref() == Some(MOUNT_ID))
        .collect()
}

/// External ids currently materialized.
async fn external_ids(env: &Env) -> Vec<String> {
    let mut ids: Vec<String> = mount_nodes(env)
        .await
        .iter()
        .filter_map(|n| str_prop(n, "__external_id"))
        .collect();
    ids.sort();
    ids
}

async fn state(env: &Env) -> MountState {
    read_mount_state(&env.storage, TENANT, REPO, BRANCH, MOUNT_ID)
        .await
        .unwrap()
        .expect("mount state")
}

async fn write_state(env: &Env, mut s: MountState) {
    persist_mount_state(&env.storage, TENANT, REPO, BRANCH, MOUNT_ID, &mut s)
        .await
        .unwrap();
}

/// The mount as the scheduler sees it, for `check::is_due`.
async fn mount_config(env: &Env) -> MountConfig {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch(BRANCH).unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    let node = tx
        .get_node(SYSTEM_WORKSPACE, MOUNT_ID)
        .await
        .unwrap()
        .unwrap();
    MountConfig::from_node(&node).unwrap()
}

fn prop_obj(v: Value) -> PropertyValue {
    serde_json::from_value(v).unwrap()
}

/// Write the `raisin:Integration` + `raisin:VirtualMount` config nodes the
/// handler reads, with a caller-chosen `max_items_per_sync` (the knob that
/// decides whether a walk chunks).
async fn persist_config_nodes(env: &Env, max_items_per_sync: u64) {
    let tx = begin(env).await;

    let mut integ = Node {
        id: "integration-e2e".to_string(),
        node_type: "raisin:Integration".to_string(),
        name: "mock".to_string(),
        path: "/integrations/mock".to_string(),
        workspace: Some(SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    for (k, v) in [
        ("title", "Mock"),
        ("provider_type", "mock"),
        ("adapter_function", "/adapters/mock"),
    ] {
        integ
            .properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    tx.upsert_deep_node(SYSTEM_WORKSPACE, &integ, "raisin:Folder")
        .await
        .unwrap();

    let mut mount = Node {
        id: MOUNT_ID.to_string(),
        node_type: "raisin:VirtualMount".to_string(),
        name: "mock".to_string(),
        path: "/mounts/mock".to_string(),
        workspace: Some(SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    for (k, v) in [
        ("title", "Mock"),
        ("integration_ref", "/integrations/mock"),
        ("target_workspace", TARGET_WS),
        ("mount_path", MOUNT_PATH),
        ("remote_root", "root"),
        ("target_branch", BRANCH),
    ] {
        mount
            .properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    mount
        .properties
        .insert("enabled".to_string(), PropertyValue::Boolean(true));
    mount.properties.insert(
        "sync_config".to_string(),
        prop_obj(json!({
            "mode": "poll",
            "interval_seconds": 300,
            "max_items_per_sync": max_items_per_sync,
        })),
    );
    tx.upsert_deep_node(SYSTEM_WORKSPACE, &mount, "raisin:Folder")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

// ---------------------------------------------------------------------------
// 1. the empty-id regression
// ---------------------------------------------------------------------------

/// A full walk over a multi-page provider must import EVERY item, each as its
/// own node with a distinct, non-empty id.
///
/// Pins the empty-id bug: `NodeService::create` does not mint an id, so every
/// materialized node was written with `id: ""`. The first one landed and the
/// rest collided with it, leaving a mount that reported hundreds of items
/// imported and had exactly one node. Counting nodes catches the collision;
/// asserting on the ids themselves catches it before it turns into a count.
#[tokio::test(flavor = "multi_thread")]
async fn a_full_walk_imports_every_item_with_distinct_ids() {
    let env = setup().await;
    persist_config_nodes(&env, 400).await;

    let provider = Arc::new(Provider::new(125, 100));
    provider.seed(250);

    let summary = sync(&env, &provider, "full").await;
    assert_eq!(summary["outcome"], json!("ok"), "run failed: {summary}");
    assert_eq!(count(&summary, "written"), 250);

    let nodes = mount_nodes(&env).await;
    assert_eq!(nodes.len(), 250, "every provider item must become a node");

    let ids: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(
        !ids.contains(""),
        "a materialized node was written with an EMPTY id"
    );
    assert_eq!(ids.len(), 250, "node ids must be unique");

    // And the mount owns one node per external id — no dedup, no overwrite.
    assert_eq!(external_ids(&env).await.len(), 250);
}

// ---------------------------------------------------------------------------
// 2. the delta-baseline regression (the one that shipped)
// ---------------------------------------------------------------------------

/// A mount whose import has finished must still receive NEW upstream items.
///
/// This is the exact production failure: the full walk captured page 1 of the
/// provider's initial ENUMERATION as its delta token instead of a resumable
/// "from now on" baseline. Every later run then resumed that enumeration,
/// re-reading mail it had just imported and reporting `0 written / N skipped`,
/// while genuinely new items waited behind a walk that could not converge.
///
/// Both assertions matter. The new node proves items arrive; `skipped == 0`
/// proves they arrived through a delta feed rather than by re-enumerating the
/// whole mailbox — a baseline regression would show up as 250 skips even in the
/// runs where the enumeration happened to reach the new item.
#[tokio::test(flavor = "multi_thread")]
async fn a_completed_walk_still_receives_a_new_item() {
    let env = setup().await;
    persist_config_nodes(&env, 400).await;

    let provider = Arc::new(Provider::new(125, 100));
    provider.seed(250);

    // Run the import to completion.
    let first = sync(&env, &provider, "full").await;
    assert_eq!(first["outcome"], json!("ok"), "run failed: {first}");
    assert!(state(&env).await.backfill_complete, "walk must finish");
    assert_eq!(mount_nodes(&env).await.len(), 250);

    // One new item appears upstream.
    provider.add_item("NEW-1", "brand-new.txt");

    let second = sync(&env, &provider, "delta").await;
    assert_eq!(second["outcome"], json!("ok"), "run failed: {second}");
    assert!(
        external_ids(&env).await.contains(&"NEW-1".to_string()),
        "a new upstream item never reached the mount: the stored delta token is \
         an enumeration cursor, not a baseline"
    );
    assert_eq!(
        count(&second, "written"),
        1,
        "the delta run must write exactly the new item"
    );
    assert_eq!(
        count(&second, "skipped"),
        0,
        "the delta run re-read items it had already imported: it resumed an \
         enumeration instead of a baseline"
    );
}

// ---------------------------------------------------------------------------
// 3. chunked backfill
// ---------------------------------------------------------------------------

/// A backfill that chunks across several runs must end up with everything, and
/// must never delete what an earlier chunk imported.
///
/// Two failure modes meet here. A walk that restarts from the top each run
/// re-imports its first `max_items_per_sync` items forever and never reaches
/// the rest (a production mailbox could not be imported at all). And a resumed
/// pass that runs reconcile deletes would remove every node the earlier chunks
/// wrote, because its `seen` set holds only the final chunk. Both are invisible
/// unless a test actually runs the chunks to completion.
#[tokio::test(flavor = "multi_thread")]
async fn a_resumed_backfill_keeps_every_earlier_chunk() {
    let env = setup().await;
    // Well below the corpus, so the walk is forced to chunk.
    persist_config_nodes(&env, 100).await;

    let provider = Arc::new(Provider::new(60, 100));
    provider.seed(250);

    let mut runs = 0;
    let mut seen_after_first_chunk: Vec<String> = Vec::new();
    loop {
        let summary = sync(&env, &provider, "full").await;
        assert_eq!(summary["outcome"], json!("ok"), "run failed: {summary}");
        assert_eq!(
            count(&summary, "deleted"),
            0,
            "a chunked walk must never delete: 'not seen' only means 'not \
             reached yet' until the walk has run end to end"
        );
        runs += 1;
        if runs == 1 {
            seen_after_first_chunk = external_ids(&env).await;
            assert!(
                !seen_after_first_chunk.is_empty(),
                "first chunk imported nothing"
            );
        }
        if state(&env).await.backfill_complete {
            break;
        }
        assert!(runs < 12, "backfill never completed after {runs} runs");
    }
    assert!(runs > 1, "max_items_per_sync did not chunk the walk");

    let ids = external_ids(&env).await;
    assert_eq!(
        ids.len(),
        provider.item_count(),
        "the finished backfill must hold every provider item"
    );
    for id in &seen_after_first_chunk {
        assert!(ids.contains(id), "item {id} from the first chunk was lost");
    }
}

// ---------------------------------------------------------------------------
// 4. steady state
// ---------------------------------------------------------------------------

/// With nothing changed upstream, a delta run writes nothing and leaves the
/// backfill counter alone.
///
/// `backfill_items_done` is documented as "the current full walk". The delta
/// path used to add to it, and since nothing in that path ever resets it, a
/// settled mount accumulated incremental work forever — the console reported
/// "433,500 items imported" for an import that had finished months earlier.
/// Delta work belongs in `delta_items_done`.
#[tokio::test(flavor = "multi_thread")]
async fn a_steady_state_delta_writes_nothing_and_leaves_the_counters_alone() {
    let env = setup().await;
    persist_config_nodes(&env, 400).await;

    let provider = Arc::new(Provider::new(125, 100));
    provider.seed(250);

    sync(&env, &provider, "full").await;
    let after_walk = state(&env).await;
    assert!(after_walk.backfill_complete);
    let backfill_done = after_walk.backfill_items_done;
    assert_eq!(backfill_done, 250);

    for run in 0..2 {
        let summary = sync(&env, &provider, "delta").await;
        assert_eq!(summary["outcome"], json!("ok"), "run failed: {summary}");
        assert_eq!(
            count(&summary, "written"),
            0,
            "run {run}: an unchanged provider must produce no writes"
        );
        assert_eq!(
            count(&summary, "deleted"),
            0,
            "run {run}: nothing to delete"
        );
        assert_eq!(
            state(&env).await.backfill_items_done,
            backfill_done,
            "run {run}: delta work inflated the BACKFILL counter"
        );
    }
    assert_eq!(mount_nodes(&env).await.len(), 250);
}

// ---------------------------------------------------------------------------
// 5. stop
// ---------------------------------------------------------------------------

/// A stop ends the run early, and once the resume point is discarded the mount
/// is no longer due — so the scheduler does not simply start another chunk on
/// the next tick.
///
/// A stop that left the mount due would not be a stop at all: an unfinished
/// backfill is deliberately due *immediately* (`check::is_due`), so the very
/// next 60s tick would re-enqueue the import the operator just stopped. The
/// clearing itself lives in the HTTP stop endpoint
/// (`raisin-transport-http/.../integrations/mount_control.rs`), which this
/// crate cannot reach; the edit is reproduced below so the interaction between
/// the two halves is pinned somewhere.
///
/// NOT asserted here, because it does not currently hold: that the stopped walk
/// leaves a usable resume point. `full.rs` says it saves one "exactly as a
/// truncated chunk does", but it never pushes the folder it was walking back
/// onto the stack and never stores `backfill_cursor` — see the report
/// accompanying these tests. Asserting the present behaviour would cement the
/// bug, so this test asserts only what must be true under either version.
#[tokio::test(flavor = "multi_thread")]
async fn stop_ends_the_run_and_a_cleared_resume_point_stops_rescheduling() {
    let env = setup().await;
    persist_config_nodes(&env, 400).await;

    let provider = Arc::new(Provider::new(60, 100));
    provider.seed(250);

    // The operator's stop lands before the run reaches its first page boundary,
    // where the walk re-reads it from the store.
    let mut s = state(&env).await;
    s.stop_requested = true;
    write_state(&env, s).await;

    let summary = sync(&env, &provider, "full").await;
    assert_eq!(summary["outcome"], json!("ok"), "run failed: {summary}");

    let stopped = state(&env).await;
    assert!(
        !stopped.backfill_complete,
        "a stopped walk did not reach the end and must not claim it did"
    );
    assert!(
        mount_nodes(&env).await.len() < provider.item_count(),
        "the walk ignored the stop and imported everything"
    );
    assert_eq!(
        count(&summary, "deleted"),
        0,
        "a partial walk must not reconcile-delete"
    );

    // What the stop endpoint writes: the resume point goes, so the scheduler
    // has nothing left to consider urgent.
    let mut cleared = state(&env).await;
    cleared.backfill_cursor = None;
    cleared.backfill_stack.clear();
    cleared.backfill_items_done = 0;
    cleared.delta_items_done = 0;
    cleared.backfill_complete = false;
    cleared.last_sync_token = None;
    write_state(&env, cleared).await;

    assert!(
        !check::is_due(&mount_config(&env).await, Utc::now().timestamp()),
        "a stopped mount was re-enqueued on the next tick"
    );
}

// ---------------------------------------------------------------------------
// 6. pause
// ---------------------------------------------------------------------------

/// A paused mount is never due, its run is skipped, and its push subscription
/// is left registered.
///
/// Pause and disable have deliberately different blast radius: disabling tears
/// the provider subscription down, so notifications arriving while it is off
/// are lost outright. A pause that also unsubscribed would silently turn a
/// temporary suspension into dropped data and a re-registration on resume.
#[tokio::test(flavor = "multi_thread")]
async fn a_paused_mount_is_never_due_and_keeps_its_subscription() {
    let env = setup().await;
    persist_config_nodes(&env, 400).await;

    let provider = Arc::new(Provider::new(125, 100));
    provider.seed(10);

    let mut s = state(&env).await;
    s.paused = true;
    s.push_subscription_id = Some("sub-1".to_string());
    s.push_status = Some("active".to_string());
    s.push_expires_at = Some("2099-01-01T00:00:00Z".to_string());
    write_state(&env, s).await;

    assert!(
        !check::is_due(&mount_config(&env).await, Utc::now().timestamp()),
        "a paused mount must never be scheduled"
    );

    let summary = sync(&env, &provider, "delta").await;
    assert_eq!(summary["outcome"], json!("skipped"));
    assert_eq!(summary["reason"], json!("paused"));
    assert!(
        mount_nodes(&env).await.is_empty(),
        "a paused mount must not materialize anything"
    );

    let after = state(&env).await;
    assert_eq!(
        after.push_subscription_id.as_deref(),
        Some("sub-1"),
        "pausing tore down the push subscription; only disabling may do that"
    );
    assert_eq!(after.push_status.as_deref(), Some("active"));
}
