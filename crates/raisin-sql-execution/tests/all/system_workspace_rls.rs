//! SQL writes into the system workspaces (`raisin:access_control`, where users
//! and roles live, and `raisin:system`, where flow instances and config live)
//! are governed by row-level security like any other workspace: a caller
//! without a write grant there changes nothing, even one who may read
//! everything.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, NodeTypeRepository, RepoScope, Storage, WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_sys_ws";
const REPO: &str = "r_sys_ws";
const BRANCH: &str = "main";
const WORKSPACES: &[&str] = &["raisin:access_control", "raisin:system"];

async fn storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = raisin_rocksdb::RocksDBStorage::new(dir.path()).unwrap();
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await;
    for ws in WORKSPACES {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .unwrap();
    }
    storage
        .node_types()
        .create(
            raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Item" })).unwrap(),
            raisin_storage::CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .unwrap();
    (Arc::new(storage), dir)
}

fn engine(
    s: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth: AuthContext,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    for ws in WORKSPACES {
        catalog.register_workspace(ws.to_string());
    }
    QueryEngine::new(s.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(auth)
}

async fn run(e: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> Result<(), String> {
    let mut stream = e.execute(sql).await.map_err(|e| e.to_string())?;
    while let Some(row) = stream.next().await {
        row.map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn marker(
    e: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    ws: &str,
    path: &str,
) -> Option<String> {
    let sql =
        format!("SELECT properties->>'marker'::String AS m FROM '{ws}' WHERE path = '{path}'");
    let mut stream = e.execute(&sql).await.unwrap();
    while let Some(row) = stream.next().await {
        if let Some(PropertyValue::String(s)) = row.unwrap().columns.get("m") {
            return Some(s.clone());
        }
    }
    None
}

/// Reads everything, in every workspace; writes nothing.
fn reader() -> AuthContext {
    let mut p = ResolvedPermissions::empty("reader");
    p.permissions = vec![Permission::new("/**", vec![Operation::Read]).with_workspace("*")];
    AuthContext::for_user("reader").with_permissions(p)
}

fn anonymous() -> AuthContext {
    let public = Permission::new("/**", vec![Operation::Read]).with_workspace("*");
    AuthContext::anonymous_user("anon")
        .with_permissions(ResolvedPermissions::anonymous(vec![public]))
}

#[tokio::test]
async fn system_workspaces_refuse_sql_writes_without_a_grant() {
    let (s, _dir) = storage().await;
    let system = engine(&s, AuthContext::system());
    for ws in WORKSPACES {
        run(
            &system,
            &format!(
                "INSERT INTO '{ws}' (id, path, node_type, properties) VALUES \
                 ('seed-{n}', '/seed', 'test:Item', '{{\"marker\":\"original\"}}'::JSONB)",
                n = ws.replace(':', "-")
            ),
        )
        .await
        .unwrap();
    }

    for who in [reader(), anonymous()] {
        let e = engine(&s, who);
        for ws in WORKSPACES {
            let _ = run(
                &e,
                &format!(
                    "INSERT INTO '{ws}' (id, path, node_type, properties) VALUES \
                     ('evil', '/evil', 'test:Item', '{{\"marker\":\"evil\"}}'::JSONB)"
                ),
            )
            .await;
            let _ = run(
                &e,
                &format!(
                    "UPDATE '{ws}' SET properties = properties || '{{\"marker\":\"hacked\"}}'::jsonb WHERE path = '/seed'"
                ),
            )
            .await;
            let _ = run(&e, &format!("DELETE FROM '{ws}' WHERE path = '/seed'")).await;

            assert_eq!(
                marker(&system, ws, "/evil").await,
                None,
                "{ws}: insert went through"
            );
            assert_eq!(
                marker(&system, ws, "/seed").await.as_deref(),
                Some("original"),
                "{ws}: update or delete went through"
            );
        }
    }
}
