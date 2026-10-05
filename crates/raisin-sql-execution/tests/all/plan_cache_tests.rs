//! The prepared-statement cache (plan Phase 13b, `engine/prepared.rs`) holds a
//! statement's analysis and optimized LOGICAL plan per `(catalog, SQL text)`.
//! These pin what it must never hold on to:
//!
//! - a schema change (a workspace created or dropped) yields a new catalog,
//!   and the statement is analyzed again against it;
//! - a NodeType change (a compound index declared and built) reaches a cached
//!   statement — the physical plan is never cached;
//! - a different caller runs the same cached statement under its OWN
//!   row-level security;
//! - the derived-cache registry (checkpoint ingest) drops it.
//!
//! The cache is process-wide, so these run one at a time (`SERIAL`).

use super::compound_index_hierarchy::{message_type, node, NODE_TYPE};
use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_rocksdb::RocksDBStorage;
use raisin_sql::analyzer::Catalog;
use raisin_sql_execution::{
    invalidate_compound_index_cache, invalidate_workspace_catalog, plan_cache_contains,
    workspace_catalog, QueryEngine,
};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CreateNodeOptions, NodeRepository,
    NodeTypeRepository, RepoScope, Storage, StorageScope, WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

pub(super) const BRANCH: &str = "main";
pub(super) const WS: &str = "ws";

pub(super) static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A repository `tenant/repo` with workspace `ws`, `NODE_TYPE` (no compound
/// index) and `/a` with three children.
pub(super) async fn fixture(tenant: &str, repo: &str) -> (Arc<RocksDBStorage>, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(tmp.path()).unwrap());
    let _ = storage
        .branches()
        .create_branch(tenant, repo, BRANCH, "t", None, None, false, false)
        .await;
    workspace(&storage, tenant, repo, WS).await;
    let mut plain = message_type();
    plain.compound_indexes = None;
    upsert_type(&storage, tenant, repo, plain).await;
    let scope = StorageScope::new(tenant, repo, BRANCH, WS);
    for (id, path, parent) in [
        ("a", "/a", "/"),
        ("m0", "/a/m0", "a"),
        ("m1", "/a/m1", "a"),
        ("m2", "/a/m2", "a"),
    ] {
        let options = CreateNodeOptions {
            validate_parent_allows_child: false,
            validate_workspace_allows_type: false,
            ..Default::default()
        };
        storage
            .nodes()
            .create(scope, node(id, path, parent), options)
            .await
            .unwrap();
    }
    (storage, tmp)
}

pub(super) async fn workspace(storage: &RocksDBStorage, tenant: &str, repo: &str, name: &str) {
    let ws = raisin_models::workspace::Workspace::new(name.to_string());
    storage
        .workspaces()
        .put(RepoScope::new(tenant, repo), ws)
        .await
        .unwrap();
}

pub(super) async fn upsert_type(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
    node_type: raisin_models::nodes::NodeType,
) {
    storage
        .node_types()
        .upsert(
            BranchScope::new(tenant, repo, BRANCH),
            node_type,
            CommitMetadata::system("t"),
        )
        .await
        .unwrap();
}

/// The repository's shared catalog — the one every transport plans against.
pub(super) async fn catalog(
    storage: &RocksDBStorage,
    tenant: &str,
    repo: &str,
) -> Arc<dyn Catalog> {
    workspace_catalog(storage, tenant, repo).await.unwrap()
}

pub(super) fn engine(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    catalog: &Arc<dyn Catalog>,
    auth: AuthContext,
) -> QueryEngine<RocksDBStorage> {
    QueryEngine::new(storage.clone(), tenant, repo, BRANCH)
        .with_catalog(catalog.clone())
        .with_auth(auth)
}

async fn ids(engine: &QueryEngine<RocksDBStorage>, sql: &str) -> Vec<String> {
    let stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("[{sql}]: {e}"));
    drain(stream, sql).await
}

/// The physical plan of `sql` from a call the cache ANSWERED. Residency is
/// no proof under eviction pressure (the whole suite shares the cache), so
/// this asks until one call reports a hit.
async fn hit_plan(engine: &QueryEngine<RocksDBStorage>, sql: &str) -> String {
    for _ in 0..50 {
        let (plan, cached) = engine.prepared_physical_plan(sql).await.unwrap();
        if cached {
            return plan;
        }
    }
    panic!("[{sql}] is never answered from the cache");
}

async fn is_cached(engine: &QueryEngine<RocksDBStorage>, sql: &str) -> bool {
    engine.prepared_physical_plan(sql).await.unwrap().1
}

#[tokio::test]
async fn a_schema_change_misses_the_cached_statement() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pc_schema", "pc_schema_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let sql = format!("SELECT id FROM '{WS}' WHERE path = '/a/m1'");

    let before = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &before, AuthContext::system());
    assert_eq!(ids(&sys, &sql).await.len(), 1);
    hit_plan(&sys, &sql).await;

    // A schema change: a new workspace, and the invalidation the workspace
    // event performs in the server.
    workspace(&storage, t, r, "news").await;
    invalidate_workspace_catalog(t, r);
    let after = catalog(&storage, t, r).await;
    assert!(
        !Arc::ptr_eq(&before, &after),
        "a schema change is a new catalog"
    );
    let sys = engine(&storage, t, r, &after, AuthContext::system());
    assert!(
        !is_cached(&sys, &sql).await,
        "the statement is analyzed again against the new catalog"
    );
    assert_eq!(ids(&sys, &sql).await.len(), 1);
    // ...and the new schema is in force: the new workspace plans.
    assert!(ids(&sys, "SELECT id FROM 'news'").await.is_empty());

    // Checkpoint ingest emits no events: the derived-cache registry drops it.
    hit_plan(&sys, &sql).await;
    raisin_core::invalidate_all_derived_caches();
    assert!(!plan_cache_contains(&after, &sql, false));
    assert!(!is_cached(&sys, &sql).await);
}

#[tokio::test]
async fn a_nodetype_change_reaches_a_cached_statement() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pc_type", "pc_type_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let cat = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &cat, AuthContext::system());
    let sql = format!(
        "SELECT name FROM '{WS}' WHERE CHILD_OF('/a') AND node_type = '{NODE_TYPE}' \
         ORDER BY created_at DESC LIMIT 3"
    );
    let rows = ids(&sys, &sql).await;
    assert_eq!(rows.len(), 3);
    let plan = hit_plan(&sys, &sql).await;
    assert!(!plan.contains("CompoundIndexScan"), "no index yet:\n{plan}");

    // The NodeType gains a compound index, which is then built.
    upsert_type(&storage, t, r, message_type()).await;
    raisin_rocksdb::management::async_indexing::rebuild_indexes(
        &storage,
        t,
        r,
        BRANCH,
        WS,
        raisin_storage::IndexType::Compound,
    )
    .await
    .unwrap();
    // What the definitions cache's 30 s TTL does on its own.
    invalidate_compound_index_cache();

    // A statement answered from the cache plans with the index: nothing
    // NodeType-derived is cached with it (the physical plan never is).
    let plan = hit_plan(&sys, &sql).await;
    assert!(
        plan.contains("CompoundIndexScan"),
        "the new index is used:\n{plan}"
    );
    assert_eq!(ids(&sys, &sql).await, rows, "same rows through the index");
}

/// Read on `ws` only when `allowed`.
pub(super) fn reader(allowed: bool) -> AuthContext {
    let permissions = if allowed {
        vec![Permission::new("/**", vec![Operation::Read]).with_workspace(WS)]
    } else {
        vec![Permission::new("/**", vec![Operation::Read]).with_workspace("elsewhere")]
    };
    AuthContext::for_user("reader").with_permissions(ResolvedPermissions {
        user_id: "reader".into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions,
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

#[tokio::test]
async fn a_different_caller_is_filtered_on_a_cached_statement() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pc_caller", "pc_caller_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let cat = catalog(&storage, t, r).await;
    for sql in [
        format!("SELECT id FROM '{WS}' WHERE path = '/a/m1'"),
        format!("SELECT * FROM '{WS}' WHERE id = 'm1'"),
        format!("SELECT id FROM '{WS}' WHERE CHILD_OF('/a')"),
    ] {
        let sys = engine(&storage, t, r, &cat, AuthContext::system());
        let all = ids(&sys, &sql).await;
        assert!(!all.is_empty(), "[{sql}]");
        // Cached for this catalog — and the key has no caller in it.
        hit_plan(&sys, &sql).await;

        // Same catalog, same text — a cache hit — and the caller's own RLS.
        let denied = engine(&storage, t, r, &cat, reader(false));
        assert!(ids(&denied, &sql).await.is_empty(), "[{sql}] leaked");
        let allowed = engine(&storage, t, r, &cat, reader(true));
        assert_eq!(ids(&allowed, &sql).await, all, "[{sql}]");

        // The batch entry point (HTTP, WS, pgwire) has its OWN cache key, so
        // the system caller primes THAT key and each reader's call must be
        // answered from it — otherwise the reader analyzes the statement
        // itself and RLS on a batch-path hit is never exercised.
        assert!(
            batch_ids_from_cache(&sys, &denied, &sql).await.is_empty(),
            "[{sql}] leaked through a cached batch statement"
        );
        assert_eq!(
            batch_ids_from_cache(&sys, &allowed, &sql).await,
            all,
            "[{sql}] batch"
        );
    }
}

/// Rows of `sql` through `engine`'s BATCH entry point, from a call the cache
/// ANSWERED with the entry `primer` put there. Retries the pair under
/// eviction pressure (the suite shares the cache).
async fn batch_ids_from_cache(
    primer: &QueryEngine<RocksDBStorage>,
    engine: &QueryEngine<RocksDBStorage>,
    sql: &str,
) -> Vec<String> {
    for _ in 0..50 {
        let (stream, _) = primer.execute_batch_sync_traced(sql).await.unwrap();
        drain(stream, sql).await;
        let (stream, cached) = engine.execute_batch_sync_traced(sql).await.unwrap();
        let rows = drain(stream, sql).await;
        if cached {
            return rows;
        }
    }
    panic!("[{sql}] batch is never answered from the cache");
}

pub(super) async fn drain(mut stream: raisin_sql_execution::RowStream, sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("[{sql}]: {e}"));
        out.push(format!("{:?}", row.columns.values().next().cloned()));
    }
    out
}
