//! A mutating ACL statement drops the cached permission sets; a read-only one does not.
//!
//! `invalidate_all_permission_caches` is documented as "call after any write to the
//! access-control workspace that can change what somebody may do". The WebSocket event
//! handler calls it for node events, but `GRANT`/`REVOKE` and the other ACL statements
//! write straight to the node repository, which publishes no event. Without an explicit
//! call, a `REVOKE GROUP` took effect only when the five-minute TTL ran out.
//!
//! Both tests register a real `PermissionCache` in the process-wide registry, which is
//! the same path the cached permission service uses.

use futures::StreamExt;
use raisin_core::services::permission_cache::PermissionCache;
use raisin_core::services::permission_cache_registry::register_permission_cache;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::ResolvedPermissions;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";
const ACCESS_CONTROL_WS: &str = "raisin:access_control";

async fn setup() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);

    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(ACCESS_CONTROL_WS.to_string()),
        )
        .await
        .expect("workspace");

    for nt in ["raisin:User", "raisin:Group", "raisin:Role"] {
        storage
            .node_types()
            .create(
                BranchScope::new(TENANT, REPO, BRANCH),
                serde_json::from_value(serde_json::json!({ "name": nt })).expect("nt"),
                CommitMetadata {
                    message: "t".into(),
                    actor: "t".into(),
                    is_system: true,
                },
            )
            .await
            .expect("nodetype");
    }

    (storage, temp_dir)
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(ACCESS_CONTROL_WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system())
}

async fn run(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
    }
}

fn cached(user_id: &str) -> ResolvedPermissions {
    ResolvedPermissions {
        user_id: user_id.into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec!["editors".into()],
        permissions: vec![],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    }
}

/// A cache with one entry for `key`, registered like the cached permission service does.
fn primed_cache(key: &str) -> Arc<PermissionCache> {
    let cache = Arc::new(PermissionCache::new(Duration::from_secs(300)));
    register_permission_cache(&cache);
    cache.put(key, cached(key));
    assert!(cache.get(key).is_some(), "precondition: entry is cached");
    cache
}

// One test, fixed order: the registry is process-wide, so a GRANT in a parallel test
// would empty this test's cache and make the read-only check fail at random.
#[tokio::test]
async fn mutating_acl_statements_drop_cached_permissions_read_only_ones_do_not() {
    let (storage, _dir) = setup().await;
    let engine = engine(&storage);
    run(
        &engine,
        "CREATE USER 'acl-cache' EMAIL 'acl-cache@example.com'",
    )
    .await;
    run(&engine, "CREATE GROUP 'editors'").await;

    let cache = primed_cache("acl-cache");
    run(&engine, "SHOW USERS").await;
    run(&engine, "DESCRIBE USER 'acl-cache'").await;
    assert!(
        cache.get("acl-cache").is_some(),
        "a read-only ACL statement must not drop cached permissions"
    );

    run(&engine, "GRANT GROUP 'editors' TO USER 'acl-cache'").await;
    assert!(
        cache.get("acl-cache").is_none(),
        "GRANT must drop cached permissions, otherwise the change waits for the TTL"
    );

    cache.put("acl-cache", cached("acl-cache"));
    run(&engine, "REVOKE GROUP 'editors' FROM USER 'acl-cache'").await;
    assert!(
        cache.get("acl-cache").is_none(),
        "REVOKE must drop cached permissions, otherwise a removed member keeps access"
    );
}
