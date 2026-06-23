//! In-process throughput baseline for RaisinDB's write/read paths.
//!
//! Measures the engine *ceiling* (no network/protocol overhead) through the real
//! stack: `NodeService` → transaction → RLS → `RocksDBStorage`. This is the
//! "maximum possible" number; protocol benchmarks (WS / pgwire) sit on top and
//! will be strictly lower.
//!
//! Run (single-threaded, show output):
//!   cargo test -p raisin-core --test throughput_baseline -- --ignored --nocapture
//!
//! Tune the workload size with `BENCH_N` (default 5000):
//!   BENCH_N=20000 cargo test -p raisin-core --test throughput_baseline -- --ignored --nocapture
//!
//! Dimensions covered here:
//!   - single-op writes (one auto-commit per node)   — per-write floor
//!   - batched writes (N nodes per txn, one commit)   — write ceiling
//!   - reads by id / by path
//!   - RLS off (system) vs RLS on (matching perm, no condition) vs RLS on
//!     (non-graph condition) — isolates ACL cost
//!
//! NOT covered here (need separate harnesses): compound-index pagination (SQL
//! layer) and the WS / pgwire protocol comparison (need a running server).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::schema::{PropertyType, PropertyValueSchema};
use raisin_models::nodes::types::NodeType;
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope,
    RepositoryManagementRepository, Storage, WorkspaceRepository,
};

const TENANT: &str = "default";
const REPO: &str = "default";
const BRANCH: &str = "main";
const WS: &str = "default";
const DOC_TYPE: &str = "bench:Doc";

fn bench_n() -> usize {
    std::env::var("BENCH_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000)
}

async fn bootstrap() -> (Arc<RocksDBStorage>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());

    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    storage
        .workspaces()
        .put(RepoScope::new(TENANT, REPO), Workspace::new(WS.to_string()))
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "bench", None, None, false, false)
        .await
        .unwrap();

    let node_type = NodeType {
        id: Some(DOC_TYPE.to_string()),
        strict: Some(false),
        name: DOC_TYPE.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(vec![PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(false),
            unique: Some(false),
            default: None,
            constraints: None,
            structure: None,
            items: None,
            value: None,
            meta: None,
            is_translatable: None,
            allow_additional_properties: None,
            index: None,
        }]),
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(false),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    };
    storage
        .node_types()
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            node_type,
            CommitMetadata::system("bench node type"),
        )
        .await
        .unwrap();

    (storage, dir)
}

/// Resolved-permissions user that actually exercises the RLS path (not system,
/// not system-admin — both of those short-circuit). Optional non-graph condition.
fn rls_user(uid: &str, condition: Option<&str>) -> AuthContext {
    let mut perm = Permission::new(
        "**",
        vec![
            Operation::Create,
            Operation::Read,
            Operation::Update,
            Operation::Delete,
        ],
    );
    if let Some(c) = condition {
        perm = perm.with_condition(c);
    }
    let resolved = ResolvedPermissions {
        user_id: uid.to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![perm],
        is_system_admin: false,
        resolved_at: None,
    };
    AuthContext::for_user(uid.to_string())
        .with_permissions(resolved)
        .with_local_user_id(uid.to_string())
}

fn doc(name: &str, owner: &str) -> Node {
    let mut props = HashMap::new();
    props.insert("title".to_string(), PropertyValue::String(name.to_string()));
    Node {
        id: String::new(),
        name: name.to_string(),
        path: String::new(),
        node_type: DOC_TYPE.to_string(),
        archetype: None,
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: Some(owner.to_string()),
        translations: None,
        tenant_id: None,
        workspace: None,
        owner_id: None,
        relations: Vec::new(),
    }
}

fn report(label: &str, n: usize, elapsed: std::time::Duration) {
    let secs = elapsed.as_secs_f64();
    let per_sec = n as f64 / secs;
    println!(
        "{:<42} {:>8} ops in {:>8.3}s  =>  {:>11.0} ops/sec  ({:>7.1} µs/op)",
        label,
        n,
        secs,
        per_sec,
        secs * 1e6 / n as f64
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "throughput baseline; run explicitly with --ignored --nocapture"]
async fn throughput_baseline() {
    let n = bench_n();
    println!("\n=== RaisinDB in-process throughput baseline (RocksDB, real NodeService path) ===");
    println!("BENCH_N = {n}   backend = RocksDBStorage (default config, temp dir)\n");

    // ---- WRITE: single-op, RLS off (system), under ONE parent (root) ----
    // Also prints a per-chunk degradation curve: if per-op cost climbs as the
    // parent accumulates children, the write path is O(children-under-parent).
    {
        let (storage, _dir) = bootstrap().await;
        let svc = NodeService::new(storage.clone()).with_auth(AuthContext::system());
        let chunk = (n / 10).max(1);
        let start = Instant::now();
        let mut chunk_start = Instant::now();
        for i in 0..n {
            svc.add_node("/", doc(&format!("s{i}"), "system"))
                .await
                .unwrap();
            if (i + 1) % chunk == 0 {
                let e = chunk_start.elapsed();
                println!(
                    "    [single-parent curve] nodes {:>7}..{:<7} {:>9.0} ops/sec ({:>7.1} µs/op)",
                    i + 1 - chunk,
                    i + 1,
                    chunk as f64 / e.as_secs_f64(),
                    e.as_secs_f64() * 1e6 / chunk as f64
                );
                chunk_start = Instant::now();
            }
        }
        report("write single-op, 1 parent (RLS off)", n, start.elapsed());
    }

    // ---- WRITE: many children under ONE non-root parent (isolates root vs non-root) ----
    {
        let (storage, _dir) = bootstrap().await;
        let svc = NodeService::new(storage.clone()).with_auth(AuthContext::system());
        svc.add_node("/", doc("oneparent", "system")).await.unwrap();
        let chunk = (n / 10).max(1);
        let start = Instant::now();
        let mut chunk_start = Instant::now();
        for i in 0..n {
            svc.add_node("/oneparent", doc(&format!("np{i}"), "system"))
                .await
                .unwrap();
            if (i + 1) % chunk == 0 {
                let e = chunk_start.elapsed();
                println!(
                    "    [non-root-parent curve] nodes {:>7}..{:<7} {:>9.0} ops/sec ({:>7.1} µs/op)",
                    i + 1 - chunk,
                    i + 1,
                    chunk as f64 / e.as_secs_f64(),
                    e.as_secs_f64() * 1e6 / chunk as f64
                );
                chunk_start = Instant::now();
            }
        }
        report(
            "write single-op, 1 NON-ROOT parent (RLS off)",
            n,
            start.elapsed(),
        );
    }

    // ---- WRITE: bucket-cap sweep (hierarchical buckets, ≤ cap children each) ----
    // Realistic hierarchical-DB write pattern: fill a bucket to `cap`, then move
    // to the next bucket. Shows sustained throughput vs bucket size — i.e. how
    // small buckets must be to hit 5k / 10k writes/sec.
    for cap in [100usize, 250, 500, 1000] {
        let (storage, _dir) = bootstrap().await;
        let svc = NodeService::new(storage.clone()).with_auth(AuthContext::system());
        let start = Instant::now();
        let mut cur_bucket = usize::MAX;
        for i in 0..n {
            let bucket = i / cap;
            if bucket != cur_bucket {
                svc.add_node("/", doc(&format!("bk{bucket}"), "system"))
                    .await
                    .unwrap();
                cur_bucket = bucket;
            }
            svc.add_node(&format!("/bk{bucket}"), doc(&format!("c{i}"), "system"))
                .await
                .unwrap();
        }
        report(
            &format!("write bucketed (≤{cap}/bucket, RLS off)"),
            n,
            start.elapsed(),
        );
    }

    // ---- WRITE: single-op, RLS on (matching perm, no condition) ----
    {
        let (storage, _dir) = bootstrap().await;
        let svc = NodeService::new(storage.clone()).with_auth(rls_user("u1", None));
        let start = Instant::now();
        for i in 0..n {
            svc.add_node("/", doc(&format!("r{i}"), "u1"))
                .await
                .unwrap();
        }
        report("write single-op (RLS on, no cond)", n, start.elapsed());
    }

    // ---- WRITE: batched (N nodes / txn, one commit), RLS off ----
    for batch in [100usize, 1000usize] {
        let (storage, _dir) = bootstrap().await;
        let svc = NodeService::new(storage.clone()).with_auth(AuthContext::system());
        let start = Instant::now();
        let mut written = 0usize;
        let mut i = 0usize;
        while written < n {
            let this = batch.min(n - written);
            let mut tx = svc.transaction();
            for _ in 0..this {
                let mut node = doc(&format!("b{i}"), "system");
                node.id = nanoid::nanoid!();
                node.path = format!("/b{i}");
                node.created_at = Some(chrono::Utc::now());
                tx.create(node);
                i += 1;
            }
            tx.commit("bench batch", "bench").await.unwrap();
            written += this;
        }
        report(
            &format!("write batched (batch={batch}, RLS off)"),
            n,
            start.elapsed(),
        );
    }

    // ---- READ setup: create N nodes, collect ids/paths ----
    let (storage, _dir) = bootstrap().await;
    let writer = NodeService::new(storage.clone()).with_auth(AuthContext::system());
    let mut ids = Vec::with_capacity(n);
    let mut paths = Vec::with_capacity(n);
    for i in 0..n {
        let created = writer
            .add_node("/", doc(&format!("d{i}"), "u1"))
            .await
            .unwrap();
        ids.push(created.id.clone());
        paths.push(created.path.clone());
    }

    // ---- READ by id: RLS off ----
    {
        let svc = NodeService::new(storage.clone()).with_auth(AuthContext::system());
        let start = Instant::now();
        for id in &ids {
            let _ = svc.get(id).await.unwrap();
        }
        report("read by-id (RLS off)", n, start.elapsed());
    }

    // ---- READ by id: RLS on, no condition ----
    {
        let svc = NodeService::new(storage.clone()).with_auth(rls_user("u1", None));
        let start = Instant::now();
        for id in &ids {
            let _ = svc.get(id).await.unwrap();
        }
        report("read by-id (RLS on, no cond)", n, start.elapsed());
    }

    // ---- READ by id: RLS on, non-graph condition (owner check) ----
    {
        let svc = NodeService::new(storage.clone()).with_auth(rls_user(
            "u1",
            Some("node.created_by == auth.local_user_id"),
        ));
        let start = Instant::now();
        for id in &ids {
            let _ = svc.get(id).await.unwrap();
        }
        report("read by-id (RLS on, owner cond)", n, start.elapsed());
    }

    // ---- READ by path: RLS off ----
    {
        let svc = NodeService::new(storage.clone()).with_auth(AuthContext::system());
        let start = Instant::now();
        for p in &paths {
            let _ = svc.get_by_path(p).await.unwrap();
        }
        report("read by-path (RLS off)", n, start.elapsed());
    }

    println!("\n(ceiling numbers — protocol layers WS/pgwire measured separately)\n");
}
