//! A path predicate plus another predicate streams in pages and stops early
//! (scale item 1): `CHILD_OF(p) AND <filter> ORDER BY __order LIMIT n` keeps
//! the scan's editorial order (no TopN over every child), and
//! `DESCENDANT_OF(p) AND node_type = t LIMIT n` stops once n rows passed.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_prefix_stream";
const REPO: &str = "r_prefix_stream";
const BRANCH: &str = "main";

type Rocks = raisin_rocksdb::RocksDBStorage;

async fn setup() -> (Arc<Rocks>, TempDir, QueryEngine<Rocks>) {
    let dir = TempDir::new().unwrap();
    let storage = Rocks::new(dir.path()).unwrap();
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "t", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new("items".into()),
        )
        .await
        .unwrap();
    for t in ["test:Folder", "test:WO", "test:Note"] {
        storage
            .node_types()
            .create(
                BranchScope::new(TENANT, REPO, BRANCH),
                serde_json::from_value(serde_json::json!({ "name": t })).unwrap(),
                CommitMetadata {
                    message: "t".into(),
                    actor: "t".into(),
                    is_system: true,
                },
            )
            .await
            .unwrap();
    }
    let storage = Arc::new(storage);
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("items".into());
    let engine = QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(raisin_models::auth::AuthContext::system());
    (storage, dir, engine)
}

async fn paths(engine: &QueryEngine<Rocks>, sql: &str) -> Vec<String> {
    let mut s = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    let mut out = Vec::new();
    while let Some(r) = s.next().await {
        let row = format!("{:?}", r.unwrap_or_else(|e| panic!("{sql}: {e}")));
        out.push(
            row.split("\"path\": String(\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string)
                .unwrap_or(row),
        );
    }
    out
}

async fn seed(engine: &QueryEngine<Rocks>) {
    paths(
        engine,
        "INSERT INTO items (path, node_type, properties) VALUES ('/wo', 'test:Folder', '{}'::JSONB)",
    )
    .await;
    let mut values = Vec::new();
    for i in 0..1500 {
        let (name, ty) = if i % 2 == 0 {
            (format!(".hidden-{i:04}"), "test:Note")
        } else {
            (format!("w-{i:04}"), "test:WO")
        };
        values.push(format!("('/wo/{name}', '{ty}', '{{}}'::JSONB)"));
    }
    for chunk in values.chunks(250) {
        paths(
            engine,
            &format!(
                "INSERT INTO items (path, node_type, properties) VALUES {}",
                chunk.join(",")
            ),
        )
        .await;
    }
}

#[tokio::test]
async fn child_of_with_a_filter_keeps_editorial_order_without_a_sort() {
    let (_s, _d, engine) = setup().await;
    seed(&engine).await;
    let q = "SELECT path FROM items WHERE CHILD_OF('/wo') AND name NOT LIKE '.%' \
             ORDER BY __order LIMIT 5";
    let plan = paths(&engine, &format!("EXPLAIN {q}")).await.join("\n");
    assert!(!plan.contains("TopN"), "{plan}");
    assert!(plan.contains("PrefixScan"), "{plan}");
    let got = paths(&engine, q).await;
    // Created in order, so editorial order is creation order.
    assert_eq!(
        got,
        vec![
            "/wo/w-0001",
            "/wo/w-0003",
            "/wo/w-0005",
            "/wo/w-0007",
            "/wo/w-0009"
        ]
    );

    // The full answer is unchanged by paging.
    let all = paths(
        &engine,
        "SELECT path FROM items WHERE CHILD_OF('/wo') AND name NOT LIKE '.%' ORDER BY __order",
    )
    .await;
    assert_eq!(all.len(), 750);
    assert_eq!(all[..5], got[..]);
}

#[tokio::test]
async fn descendant_of_with_a_type_filter_returns_complete_pages() {
    let (_s, _d, engine) = setup().await;
    seed(&engine).await;
    let five = paths(
        &engine,
        "SELECT path FROM items WHERE DESCENDANT_OF('/wo') AND node_type = 'test:WO' LIMIT 5",
    )
    .await;
    assert_eq!(five.len(), 5, "{five:?}");
    let all = paths(
        &engine,
        "SELECT path FROM items WHERE DESCENDANT_OF('/wo') AND node_type = 'test:WO'",
    )
    .await;
    assert_eq!(
        all.len(),
        750,
        "every page of the subtree is read when unbounded"
    );
}
