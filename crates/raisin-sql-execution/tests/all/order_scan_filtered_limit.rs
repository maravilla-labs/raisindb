//! `ORDER BY <property> LIMIT n` with a filter the index cannot answer must
//! return the first n MATCHING rows, never a subset.
//!
//! The order scan read one bounded batch of index entries (n x 100) and
//! filtered it, so matches beyond the batch were silently dropped: `LIMIT 5`
//! found 0 of 76 work orders `in_progress` on a load-test dataset while `LIMIT 100`
//! found all 76.

use futures::StreamExt;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope, Storage,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_order_filter";
const REPO: &str = "r_order_filter";
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
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:WO" })).unwrap(),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .unwrap();
    let storage = Arc::new(storage);
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace("items".into());
    let engine = QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_auth(raisin_models::auth::AuthContext::system());
    (storage, dir, engine)
}

async fn rows(engine: &QueryEngine<Rocks>, sql: &str) -> Vec<String> {
    let mut s = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    let mut out = Vec::new();
    while let Some(r) = s.next().await {
        let row = format!("{:?}", r.unwrap_or_else(|e| panic!("{sql}: {e}")));
        // The `path` column when present (the row's column map is unordered).
        let path = row
            .split("\"path\": String(\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .map(str::to_string);
        out.push(path.unwrap_or(row));
    }
    out
}

#[tokio::test]
async fn a_small_limit_returns_the_first_matching_rows() {
    let (_s, _d, engine) = setup().await;
    // 1,200 rows; only the 6 with the LOWEST scheduled_at are in progress,
    // so in DESC order every match sits after the first batch (5 x 100).
    let mut values = Vec::new();
    for i in 0..1200 {
        let status = if i < 6 { "in_progress" } else { "open" };
        values.push(format!(
            "('/wo-{i:04}', 'test:WO', '{{\"status\":\"{status}\",\"scheduled_at\":\"2026-{:02}-{:02}T{:02}:00\"}}'::JSONB)",
            1 + i / 112,
            1 + (i / 4) % 28,
            i % 24
        ));
    }
    for chunk in values.chunks(200) {
        rows(
            &engine,
            &format!(
                "INSERT INTO items (path, node_type, properties) VALUES {}",
                chunk.join(",")
            ),
        )
        .await;
    }

    let q = |limit: usize| {
        format!(
            "SELECT path FROM items WHERE node_type LIKE 'test:WO' \
             AND COALESCE(properties->>'status'::String, '') = 'in_progress' \
             ORDER BY properties->>'scheduled_at' DESC LIMIT {limit}"
        )
    };
    let all = rows(&engine, &q(100)).await;
    assert_eq!(all.len(), 6, "{all:?}");
    let first5 = rows(&engine, &q(5)).await;
    assert_eq!(
        first5,
        all[..5].to_vec(),
        "LIMIT 5 must be the first five of the full answer"
    );
    let plan = rows(&engine, &format!("EXPLAIN {}", q(5))).await;
    eprintln!("plan: {plan:?}");
}

/// A range on the ORDER property seeks to its bound and still returns the
/// first matching rows in order.
#[tokio::test]
async fn a_range_on_the_order_property_seeks_and_stays_exact() {
    let (_s, _d, engine) = setup().await;
    let mut values = Vec::new();
    for i in 0..600 {
        values.push(format!(
            "('/a-{i:04}', 'test:WO', '{{\"code\":\"k{i:04}\",\"status\":\"{}\"}}'::JSONB)",
            if i % 3 == 0 { "x" } else { "y" }
        ));
    }
    for chunk in values.chunks(200) {
        rows(
            &engine,
            &format!(
                "INSERT INTO items (path, node_type, properties) VALUES {}",
                chunk.join(",")
            ),
        )
        .await;
    }
    let got = rows(
        &engine,
        "SELECT path FROM items WHERE node_type LIKE 'test:WO' \
         AND properties->>'code'::String >= 'k0500' AND properties->>'code'::String < 'k0510' \
         AND COALESCE(properties->>'status'::String, '') = 'x' \
         ORDER BY properties->>'code' ASC LIMIT 3",
    )
    .await;
    assert_eq!(got, vec!["/a-0501", "/a-0504", "/a-0507"]);
}

/// A range on a `Date` property (indexed by instant, not text) finds its rows.
#[tokio::test]
async fn a_range_on_a_date_property_finds_its_rows() {
    let (storage, _d, engine) = setup().await;
    storage
        .node_types()
        .create(
            BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({
                "name": "test:TE",
                "properties": [{ "name": "work_date", "type": "Date" }]
            }))
            .unwrap(),
            CommitMetadata {
                message: "t".into(),
                actor: "t".into(),
                is_system: true,
            },
        )
        .await
        .unwrap();
    let mut values = Vec::new();
    for day in 1..=30 {
        values.push(format!(
            "('/te-{day:02}', 'test:TE', '{{\"work_date\":\"2026-11-{day:02}T00:00:00+00:00\"}}'::JSONB)"
        ));
    }
    rows(
        &engine,
        &format!(
            "INSERT INTO items (path, node_type, properties) VALUES {}",
            values.join(",")
        ),
    )
    .await;
    // `->>` compares TEXT: '2026-11-17T00:00:00+00:00' sorts before
    // '2026-11-17T00:00:00Z' ('+' < 'Z'), so that bound excludes the 17th,
    // the same answer a type scan with the same filter gives.
    for (lower, expected) in [
        ("2026-11-17", 14),
        ("2026-11-17T00:00:00Z", 13),
        ("2026-11-25", 6),
    ] {
        let got = rows(
            &engine,
            &format!(
                "SELECT path FROM items WHERE properties->>'work_date' >= '{lower}' \
                 AND node_type LIKE 'test:TE'"
            ),
        )
        .await;
        assert_eq!(got.len(), expected, "{lower}: {got:?}");
    }
    let window = rows(
        &engine,
        "SELECT path FROM items WHERE properties->>'work_date' >= '2026-11-17' \
         AND properties->>'work_date' < '2026-11-20' AND node_type LIKE 'test:TE'",
    )
    .await;
    assert_eq!(window.len(), 3, "{window:?}");
}
