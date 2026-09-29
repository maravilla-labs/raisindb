//! `RESOLVE()` end to end, over the real storage stack.
//!
//! RESOLVE used to convert its argument to `PropertyValue` and back, and to
//! refuse to resolve a node id a second time. The first made a resolved page
//! cost more than the reads behind it; the second returned an asset that
//! appears both on a page and on a teased child page resolved in one place and
//! as a bare reference in the other. These pin the replacement: references
//! nested anywhere resolve, shared ones resolve everywhere, `fields` trims the
//! inlined nodes.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test_tenant";
const REPO: &str = "test_repo";
const BRANCH: &str = "main";

async fn setup() -> (QueryEngine<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(temp_dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    let storage = Arc::new(storage);
    let mut catalog = StaticCatalog::default_nodes_schema();
    for ws in ["pages", "assets"] {
        storage
            .workspaces()
            .put(
                RepoScope::new(TENANT, REPO),
                raisin_models::workspace::Workspace::new(ws.to_string()),
            )
            .await
            .expect("workspace");
        catalog.register_workspace(ws.to_string());
    }
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(json!({ "name": "test:Doc" })).expect("nt"),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .expect("nodetype");
    let engine = QueryEngine::new(storage, TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(AuthContext::system());
    (engine, temp_dir)
}

async fn rows(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> Vec<Value> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        out.push(serde_json::to_value(&row.columns).expect("row as json"));
    }
    out
}

async fn insert(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    ws: &str,
    path: &str,
    props: Value,
) -> String {
    let props = props.to_string().replace('\'', "''");
    rows(
        engine,
        &format!(
            "INSERT INTO {ws} (path, node_type, properties) VALUES ('{path}', 'test:Doc', '{props}'::jsonb)"
        ),
    )
    .await;
    let found = rows(
        engine,
        &format!("SELECT id FROM {ws} WHERE path = '{path}'"),
    )
    .await;
    found[0]["id"].as_str().expect("id").to_string()
}

fn reference(id: &str, ws: &str, path: &str) -> Value {
    json!({ "raisin:ref": id, "raisin:workspace": ws, "raisin:path": path })
}

/// A page with an asset in a nested block and a teaser linking to a child
/// page that uses the SAME asset.
async fn seed(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    let img = insert(
        engine,
        "assets",
        "/hero.jpg",
        json!({ "alt": "A plane", "extracted_text": "long text" }),
    )
    .await;
    let child = insert(
        engine,
        "pages",
        "/child",
        json!({ "title": "Child", "image": reference(&img, "assets", "/hero.jpg") }),
    )
    .await;
    insert(
        engine,
        "pages",
        "/home",
        json!({
            "title": "Home",
            "blocks": [
                { "kind": "hero", "media": { "image": reference(&img, "assets", "/hero.jpg") } },
                { "kind": "teaser", "link": reference(&child, "pages", "/child") },
                { "kind": "broken", "link": reference("missing", "pages", "/gone") }
            ]
        }),
    )
    .await;
}

async fn home(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, projection: &str) -> Value {
    let found = rows(
        engine,
        &format!("SELECT {projection} AS p FROM pages WHERE path = '/home'"),
    )
    .await;
    found[0]["p"].clone()
}

#[tokio::test]
async fn resolve_inlines_nested_and_shared_references() {
    let (engine, _dir) = setup().await;
    seed(&engine).await;

    let p = home(&engine, "RESOLVE(properties, 2)").await;
    assert_eq!(p["title"], "Home");
    assert_eq!(p["blocks"][0]["media"]["image"]["alt"], "A plane");
    assert_eq!(p["blocks"][0]["media"]["image"]["path"], "/hero.jpg");
    assert_eq!(p["blocks"][1]["link"]["title"], "Child");
    // The asset the child page uses is the one already inlined above: it must
    // be resolved here too, not left as a bare reference.
    assert_eq!(p["blocks"][1]["link"]["image"]["alt"], "A plane");
    // An unresolvable reference is kept verbatim.
    assert_eq!(p["blocks"][2]["link"]["raisin:ref"], "missing");

    // Depth 1 stops at the first level.
    let p = home(&engine, "RESOLVE(properties)").await;
    assert_eq!(p["blocks"][1]["link"]["title"], "Child");
    assert!(p["blocks"][1]["link"]["image"]["raisin:ref"].is_string());
}

#[tokio::test]
async fn resolve_fields_trims_inlined_nodes() {
    let (engine, _dir) = setup().await;
    seed(&engine).await;

    // `fields` applies at every level, so the child page's `image` has to be
    // named for its reference to survive and be resolved.
    let p = home(&engine, "RESOLVE(properties, 2, 'alt, title, image')").await;
    let image = p["blocks"][0]["media"]["image"].as_object().expect("image");
    let mut keys: Vec<&str> = image.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["alt", "id", "name", "node_type", "path"]);
    assert_eq!(p["blocks"][1]["link"]["title"], "Child");
    assert_eq!(p["blocks"][1]["link"]["image"]["alt"], "A plane");
    assert!(p["blocks"][1]["link"]["image"]
        .get("extracted_text")
        .is_none());

    let p = home(&engine, "RESOLVE(properties, 2, 'title')").await;
    assert_eq!(p["blocks"][1]["link"]["title"], "Child");
    assert!(p["blocks"][1]["link"].get("image").is_none());
}

async fn explain(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> String {
    let found = rows(engine, &format!("EXPLAIN {sql}")).await;
    found[0]["QUERY PLAN"].as_str().expect("plan").to_string()
}

/// A lookup by key names its type too — `node_type = … AND url = …` — and the
/// key, not the type, must drive the scan whichever order the WHERE spells them.
#[tokio::test]
async fn a_property_value_drives_the_scan_over_a_node_type() {
    let (engine, _dir) = setup().await;
    seed(&engine).await;

    for sql in [
        "SELECT id FROM pages WHERE node_type = 'test:Doc' AND properties->>'title'::String = 'Child'",
        "SELECT id FROM pages WHERE properties->>'title'::String = 'Child' AND node_type = 'test:Doc'",
    ] {
        let plan = explain(&engine, sql).await;
        assert!(plan.contains("PropertyIndexScan: title=Child"), "{sql}\n{plan}");
        let found = rows(&engine, sql).await;
        assert_eq!(found.len(), 1, "{sql}");
    }

    // The type equality is still enforced.
    let found = rows(
        &engine,
        "SELECT id FROM pages WHERE node_type = 'other:Type' AND properties->>'title'::String = 'Child'",
    )
    .await;
    assert!(found.is_empty());
}

/// `node_type IN (…)` next to a predicate that already locates the rows is a
/// filter. Expanding it into a Union planned the same path lookup, or walked
/// the same subtree, once per listed type.
#[tokio::test]
async fn a_type_set_beside_a_locating_predicate_stays_a_filter() {
    let (engine, _dir) = setup().await;
    seed(&engine).await;

    for sql in [
        "SELECT id FROM pages WHERE path = '/child' AND node_type IN ('test:Doc', 'other:Type')",
        "SELECT id FROM pages WHERE CHILD_OF('/') AND node_type IN ('test:Doc', 'other:Type')",
        "SELECT id FROM pages WHERE properties->>'title'::String = 'Child' AND node_type IN ('test:Doc', 'other:Type')",
    ] {
        let plan = explain(&engine, sql).await;
        assert!(!plan.contains("Union"), "{sql}\n{plan}");
        assert_eq!(rows(&engine, sql).await.len(), if sql.contains("CHILD_OF") { 2 } else { 1 }, "{sql}");
    }

    // A set of paths still fans out into point lookups, with the type set as
    // the filter on each.
    let sql = "SELECT id FROM pages WHERE node_type IN ('test:Doc', 'other:Type') AND path IN ('/child', '/home')";
    let plan = explain(&engine, sql).await;
    assert_eq!(plan.matches("PathIndexScan").count(), 2, "{plan}");
    assert_eq!(rows(&engine, sql).await.len(), 2);
}

async fn seed_folders(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>) {
    for folder in ["/f1", "/f2", "/f3"] {
        insert(engine, "pages", folder, json!({ "title": folder })).await;
    }
    for (path, title) in [
        ("/f1/a", "A"),
        ("/f1/b", "B"),
        ("/f2/c", "C"),
        ("/f3/d", "D"),
    ] {
        insert(
            engine,
            "pages",
            path,
            json!({ "title": title, "meta": { "n": 1, "tags": ["x"] } }),
        )
        .await;
    }
}

fn sorted_paths(found: &[Value]) -> Vec<String> {
    let mut paths: Vec<String> = found
        .iter()
        .map(|r| r["path"].as_str().expect("path").to_string())
        .collect();
    paths.sort();
    paths
}

/// An OR of subtrees plans as one bounded scan per subtree, returns each row
/// once even where the disjuncts overlap, and keeps the rest of the WHERE.
#[tokio::test]
async fn an_or_of_subtrees_is_a_union_of_bounded_scans() {
    let (engine, _dir) = setup().await;
    seed_folders(&engine).await;

    let sql = "SELECT path FROM pages WHERE (CHILD_OF('/f1') OR CHILD_OF('/f2')) AND node_type = 'test:Doc'";
    let plan = explain(&engine, sql).await;
    assert!(plan.contains("Union: 2 branch(es)"), "{plan}");
    assert_eq!(plan.matches("PrefixScan").count(), 2, "{plan}");
    assert!(
        !plan.contains("TableScan") && !plan.contains("PropertyIndexScan"),
        "{plan}"
    );
    assert_eq!(
        sorted_paths(&rows(&engine, sql).await),
        ["/f1/a", "/f1/b", "/f2/c"]
    );

    // Overlapping disjuncts: every row once.
    let sql = "SELECT path FROM pages WHERE CHILD_OF('/f1') OR DESCENDANT_OF('/f1')";
    assert_eq!(sorted_paths(&rows(&engine, sql).await), ["/f1/a", "/f1/b"]);

    // A disjunct carrying its own conjunct, and one that is NULL for most rows.
    let sql = "SELECT path FROM pages WHERE CHILD_OF('/f2') \
               OR (CHILD_OF('/f3') AND node_type = 'test:Doc') \
               OR properties->>'title'::String = 'A'";
    assert_eq!(
        sorted_paths(&rows(&engine, sql).await),
        ["/f1/a", "/f2/c", "/f3/d"]
    );

    // The rest of the WHERE still applies to every branch.
    let sql = "SELECT path FROM pages WHERE (CHILD_OF('/f1') OR CHILD_OF('/f2')) AND node_type = 'other:Type'";
    assert!(rows(&engine, sql).await.is_empty());
}

/// Extracting single members of `properties` gives the same values as before
/// the per-member fast path.
#[tokio::test]
async fn json_member_extraction_reads_single_members() {
    let (engine, _dir) = setup().await;
    seed_folders(&engine).await;

    let found = rows(
        &engine,
        "SELECT properties->>'title' AS t, properties->'meta' AS m, properties->>'meta' AS mt, \
         properties->>'missing' AS x, properties AS p FROM pages WHERE path = '/f1/a'",
    )
    .await;
    let row = &found[0];
    assert_eq!(row["t"], "A");
    assert_eq!(row["m"], row["p"]["meta"]);
    let mt: Value = serde_json::from_str(row["mt"].as_str().expect("text")).expect("json text");
    assert_eq!(mt, row["p"]["meta"]);
    assert!(row.get("x").is_none() || row["x"].is_null());
}

/// A listing that projects no property skips decoding them; a filter or an
/// output column that reads one still sees them.
#[tokio::test]
async fn property_filters_still_apply_when_only_path_is_selected() {
    let (engine, _dir) = setup().await;
    seed_folders(&engine).await;

    let sql = "SELECT path FROM pages WHERE CHILD_OF('/f1') AND properties->>'title'::String = 'A'";
    assert_eq!(sorted_paths(&rows(&engine, sql).await), ["/f1/a"]);
    let sql =
        "SELECT path FROM pages WHERE DESCENDANT_OF('/f1') AND properties->>'title'::String = 'B'";
    assert_eq!(sorted_paths(&rows(&engine, sql).await), ["/f1/b"]);

    let found = rows(
        &engine,
        "SELECT path, properties FROM pages WHERE CHILD_OF('/f2')",
    )
    .await;
    assert_eq!(found[0]["properties"]["title"], "C");
    let found = rows(
        &engine,
        "SELECT path, properties->>'title' AS t FROM pages WHERE DESCENDANT_OF('/f2')",
    )
    .await;
    assert_eq!(found[0]["t"], "C");
    let found = rows(
        &engine,
        "SELECT path, name, id FROM pages WHERE CHILD_OF('/f2')",
    )
    .await;
    assert_eq!(found[0]["path"], "/f2/c");
    assert_eq!(found[0]["name"], "c");
}
