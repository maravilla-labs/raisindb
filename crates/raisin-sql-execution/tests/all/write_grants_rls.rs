//! Write grants enforced at the write layer, reached here through SQL DML:
//! a role that may update only some fields of a type, and a role that may
//! create only under its own home (a create `condition` over the new node).

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeRepository, NodeTypeRepository, RepoScope,
    Storage, StorageScope, WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

pub(crate) const TENANT: &str = "t_write_grants";
pub(crate) const REPO: &str = "r_write_grants";
pub(crate) const BRANCH: &str = "main";
pub(crate) const WS: &str = "items";

pub(crate) type Rocks = raisin_rocksdb::RocksDBStorage;

pub(crate) async fn setup() -> (Arc<Rocks>, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let storage = Rocks::new(dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:WorkOrder" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    (Arc::new(storage), dir)
}

pub(crate) fn engine(storage: &Arc<Rocks>, auth: AuthContext) -> QueryEngine<Rocks> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(auth)
}

pub(crate) fn user(id: &str, permissions: Vec<Permission>, home: Option<&str>) -> AuthContext {
    let mut auth = AuthContext::for_user(id).with_permissions(ResolvedPermissions {
        user_id: id.into(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions,
        is_system_admin: false,
        home: home.map(str::to_string),
        resolved_at: Some(std::time::Instant::now()),
    });
    auth.home = home.map(str::to_string);
    auth
}

pub(crate) async fn exec(engine: &QueryEngine<Rocks>, sql: &str) -> Result<(), String> {
    let mut stream = engine.execute(sql).await.map_err(|e| e.to_string())?;
    while let Some(row) = stream.next().await {
        row.map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn insert(engine: &QueryEngine<Rocks>, path: &str, props: &str) -> Result<(), String> {
    exec(
        engine,
        &format!(
            "INSERT INTO items (path, node_type, properties) VALUES \
             ('{path}', 'test:WorkOrder', '{props}'::JSONB)"
        ),
    )
    .await
}

async fn prop(storage: &Rocks, path: &str, key: &str) -> Option<String> {
    let node = storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
        .await
        .expect("read")?;
    node.properties.get(key).map(|v| format!("{v:?}"))
}

#[tokio::test]
async fn a_role_may_update_only_its_fields() {
    let (storage, _dir) = setup().await;
    let system = engine(&storage, AuthContext::system());
    insert(&system, "/wo-1", r#"{"status":"open","price":"100"}"#)
        .await
        .expect("seed");

    let mut grant = Permission::new("/**", vec![Operation::Read, Operation::Update]);
    grant.node_types = Some(vec!["test:WorkOrder".into()]);
    grant.fields = Some(vec!["status".into(), "notes".into()]);
    let tech = engine(&storage, user("tech", vec![grant], None));

    exec(
        &tech,
        "UPDATE items SET properties = properties || '{\"status\":\"done\",\"notes\":\"ok\"}'::JSONB \
         WHERE path = '/wo-1'",
    )
    .await
    .expect("status and notes are writable");
    assert!(prop(&storage, "/wo-1", "status")
        .await
        .unwrap()
        .contains("done"));

    let err = exec(
        &tech,
        "UPDATE items SET properties = properties || '{\"price\":\"1\"}'::JSONB WHERE path = '/wo-1'",
    )
    .await
    .expect_err("price is not writable");
    assert!(err.contains("price"), "{err}");
    assert!(prop(&storage, "/wo-1", "price")
        .await
        .unwrap()
        .contains("100"));

    // Re-stating a protected value unchanged is not a write of it.
    exec(
        &tech,
        "UPDATE items SET properties = '{\"status\":\"open\",\"notes\":\"ok\",\"price\":\"100\"}'::JSONB \
         WHERE path = '/wo-1'",
    )
    .await
    .expect("unchanged price passes");
}

#[tokio::test]
async fn a_role_may_create_only_under_its_own_home() {
    let (storage, _dir) = setup().await;
    let system = engine(&storage, AuthContext::system());
    for p in ["/customers", "/customers/acme", "/customers/other"] {
        insert(&system, p, "{}").await.expect("seed folder");
    }

    let mut grant = Permission::new("/customers/**", vec![Operation::Create, Operation::Read]);
    grant.condition = Some("node.path.startsWith(auth.home)".into());
    let customer = engine(
        &storage,
        user("acme-user", vec![grant], Some("/customers/acme")),
    );

    insert(&customer, "/customers/acme/req-1", r#"{"title":"a"}"#)
        .await
        .expect("own home");
    let err = insert(&customer, "/customers/other/req-1", r#"{"title":"b"}"#)
        .await
        .expect_err("another home");
    assert!(err.to_lowercase().contains("cannot create"), "{err}");
}

/// An update grant's condition is judged on the node AS STORED: a role that
/// may edit everything on its own work orders cannot make one its own and
/// change the price in the same write.
#[tokio::test]
async fn a_role_cannot_satisfy_an_update_condition_by_writing_it() {
    let (storage, _dir) = setup().await;
    let system = engine(&storage, AuthContext::system());
    insert(&system, "/wo-2", r#"{"owner":"someone","price":"100"}"#)
        .await
        .expect("seed");

    let mut own = Permission::new("/**", vec![Operation::Read, Operation::Update]);
    own.condition = Some("node.owner == auth.user_id".into());
    let mut notes = Permission::new("/**", vec![Operation::Read, Operation::Update]);
    notes.fields = Some(vec!["notes".into()]);
    let tech = engine(&storage, user("tech", vec![own, notes], None));

    let err = exec(
        &tech,
        "UPDATE items SET properties = properties || '{\"owner\":\"tech\",\"price\":\"1\"}'::JSONB \
         WHERE path = '/wo-2'",
    )
    .await
    .expect_err("escalation refused");
    assert!(err.contains("Cannot write property"), "{err}");
    assert!(prop(&storage, "/wo-2", "price")
        .await
        .unwrap()
        .contains("100"));
}

/// Defence in depth: even under a role that grants unrestricted update of
/// the own user node (an old `authenticated_user`), a user cannot write their
/// own roles or groups through SQL — the same write layer REST, WebSocket and
/// functions reach.
#[tokio::test]
async fn a_user_cannot_grant_themselves_roles_whatever_the_role_says() {
    const ACL: &str = "raisin:access_control";
    let (storage, _dir) = setup().await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(ACL.to_string()),
        )
        .await
        .expect("acl workspace");
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "raisin:User" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("user type");
    let acl_engine = |auth: AuthContext| {
        let mut catalog = StaticCatalog::default_nodes_schema();
        catalog.register_workspace(ACL.to_string());
        QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
            .with_catalog(Arc::new(catalog))
            .with_auth(auth)
    };
    exec(
        &acl_engine(AuthContext::system()),
        "INSERT INTO \"raisin:access_control\" (id, path, node_type, properties) VALUES \
         ('node-me', '/u_me', 'raisin:User', '{\"display_name\":\"Me\",\"roles\":[]}'::JSONB)",
    )
    .await
    .expect("seed user");

    let mut open_self = Permission::new("/**", vec![Operation::Read, Operation::Update]);
    open_self.node_types = Some(vec!["raisin:User".into()]);
    open_self.condition = Some("node.id == auth.local_user_id".into());
    let mut me = user("id-me", vec![open_self], Some("/u_me"));
    me.local_user_id = Some("node-me".into());
    let me = acl_engine(me);

    let err = exec(
        &me,
        "UPDATE \"raisin:access_control\" SET properties = properties || '{\"roles\":[\"admin\"]}'::JSONB \
         WHERE path = '/u_me'",
    )
    .await
    .expect_err("self-granted role refused");
    assert!(err.contains("roles"), "{err}");
    exec(
        &me,
        "UPDATE \"raisin:access_control\" SET properties = properties || '{\"display_name\":\"New\"}'::JSONB \
         WHERE path = '/u_me'",
    )
    .await
    .expect("display_name is the user's");
}
