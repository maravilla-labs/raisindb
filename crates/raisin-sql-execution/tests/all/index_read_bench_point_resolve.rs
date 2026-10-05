//! Scenarios R and L of the read bench (plan Phase 13b), release build:
//!
//! ```bash
//! cargo test --release -p raisin-sql-execution --test all index_read_bench_point_resolve -- --ignored --nocapture
//! ```
//!
//! - **R** — `RESOLVE(properties, 1|2)` over a page with 50 references (each
//!   target references one more node, so depth 2 inlines 100), against the
//!   storage floor: the same nodes read with `get_many_for_read`, one batch
//!   per level.
//! - **L** — a localized path lookup at depth 3 and 6 (`WHERE locale = 'fr'
//!   AND __localized_path = …`, and `RESOLVE_PATH`), against the storage
//!   lookup (`LocalizedNameSource::resolve` + `get`) and the canonical
//!   `WHERE path = …` of the same node. The index is built (`Ready`) first,
//!   and the bench asserts it answers (`served_by = index`).

use super::index_read_bench::{
    bootstrap, insert_many, run, unlock_skip_unchanged, Store, BRANCH, REPO, TENANT, WS,
};
use super::index_read_bench_point::{doc, pct, record_pct};
use raisin_context::RepositoryConfig;
use raisin_models::auth::AuthContext;
use raisin_rocksdb::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::localized::LocalizedServedBy;
use raisin_storage::{
    BatchReadItem, BranchRepository, BranchScope, CommitMetadata, NodeRepository,
    NodeTypeRepository, ReadOpts, RegistryRepository, RepoScope, RepositoryManagementRepository,
    Storage, StorageScope, WorkspaceRepository,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

fn reference(id: &str) -> Value {
    json!({ "raisin:ref": id, "raisin:workspace": WS })
}

/// Scenario R: RESOLVE over 50 references, depth 1 and 2.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "read-path baseline; run with --release --ignored --nocapture"]
async fn index_read_bench_point_resolve_r_fifty_references() {
    let (engine, storage, _dir) = bootstrap().await;
    let mut rows: Vec<(String, String, Value)> = Vec::new();
    for i in 0..50 {
        rows.push((format!("img{i}"), format!("/img{i}"), doc(1000 + i)));
        let mut target = doc(i);
        target["image"] = reference(&format!("img{i}"));
        rows.push((format!("t{i}"), format!("/t{i}"), target));
    }
    insert_many(&engine, &rows).await;
    let items: Vec<Value> = (0..50).map(|i| reference(&format!("t{i}"))).collect();
    let page = json!({ "title": "Listing", "items": items });
    insert_many(&engine, &[("page".into(), "/page".into(), page)]).await;

    let scope = BranchScope::new(TENANT, REPO, BRANCH);
    let level = |prefix: &str| -> Vec<BatchReadItem> {
        (0..50)
            .map(|i| BatchReadItem::id(WS, format!("{prefix}{i}")))
            .collect()
    };
    let (targets, images) = (level("t"), level("img"));
    let floor = |depth: usize| {
        let storage = storage.clone();
        let (targets, images) = (targets.clone(), images.clone());
        move || {
            let storage = storage.clone();
            let (targets, images) = (targets.clone(), images.clone());
            async move {
                let head = storage
                    .branches()
                    .get_branch(TENANT, REPO, BRANCH)
                    .await
                    .unwrap()
                    .unwrap()
                    .head;
                let ws = StorageScope::new(TENANT, REPO, BRANCH, WS);
                assert!(storage
                    .nodes()
                    .get(ws, "page", Some(&head))
                    .await
                    .unwrap()
                    .is_some());
                let snap = storage.nodes().open_read_snapshot();
                let opts = ReadOpts::default();
                let got = storage
                    .nodes()
                    .get_many_for_read(scope, &targets, &head, snap.as_ref(), opts.clone())
                    .await
                    .unwrap();
                assert!(got.iter().all(Option::is_some));
                if depth == 2 {
                    let got = storage
                        .nodes()
                        .get_many_for_read(scope, &images, &head, snap.as_ref(), opts)
                        .await
                        .unwrap();
                    assert!(got.iter().all(Option::is_some));
                }
            }
        }
    };
    for depth in [1usize, 2] {
        let base = pct(&format!("storage_resolve_floor_{depth}"), floor(depth)).await;
        let params = json!({ "refs": 50, "depth": depth, "inlined": 50 * depth });
        record_pct("R_storage_floor", params.clone(), base, None);
        let sql =
            format!("SELECT RESOLVE(properties, {depth}) AS r FROM '{WS}' WHERE path = '/page'");
        assert_eq!(run(&engine, &sql).await, 1);
        let took = pct(&format!("sql_resolve_{depth}"), || async {
            run(&engine, &sql).await;
        })
        .await;
        record_pct("R_sql_resolve", params, took, Some(base));
    }
}

fn localized_config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".into(), "fr".into()],
        default_branch: BRANCH.to_string(),
        ..RepositoryConfig::default()
    }
}

/// A repository with French, `SIBLINGS` siblings per level and one chain of
/// depth 6 whose every node has a French name.
async fn localized_fixture() -> (QueryEngine<Store>, Arc<Store>, tempfile::TempDir) {
    const SIBLINGS: usize = 20;
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(Store::new(dir.path()).unwrap());
    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(TENANT, REPO, localized_config())
        .await
        .unwrap();
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "bench", None, None, false, false)
        .await;
    let ws = raisin_models::workspace::Workspace::new(WS.to_string());
    storage
        .workspaces()
        .put(RepoScope::new(TENANT, REPO), ws)
        .await
        .unwrap();
    let nt = serde_json::from_value(json!({ "name": "bench:Doc" })).unwrap();
    let meta = CommitMetadata::system("bench types");
    let scope = BranchScope::new(TENANT, REPO, BRANCH);
    storage.node_types().create(scope, nt, meta).await.unwrap();
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(storage.clone(), TENANT, REPO, BRANCH)
        .with_catalog(Arc::new(catalog))
        .with_repository_config(localized_config())
        .with_auth(AuthContext::system());
    unlock_skip_unchanged(&storage).await;

    let mut parent = String::new();
    for level in 1..=6 {
        let mut rows = Vec::new();
        for s in 0..SIBLINGS {
            let name = if s == 0 {
                format!("l{level}")
            } else {
                format!("x{level}-{s}")
            };
            let id = format!("{name}-n");
            rows.push((id, format!("{parent}/{name}"), doc(level * 100 + s)));
        }
        insert_many(&engine, &rows).await;
        parent = format!("{parent}/l{level}");
        let sql = format!(
            "UPDATE {WS} FOR LOCALE 'fr' SET __node_name = 'f{level}' WHERE path = '{parent}'"
        );
        run(&engine, &sql).await;
    }
    let options = RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    };
    run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::LocalizedNames,
        options,
    )
    .await
    .expect("localized_names build");
    (engine, storage, dir)
}

/// Scenario L: localized path lookup at depth 3 and 6.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "read-path baseline; run with --release --ignored --nocapture"]
async fn index_read_bench_point_resolve_l_localized_path() {
    let (engine, storage, _dir) = localized_fixture().await;
    let source = storage.localized_names().expect("localized names");
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);
    for depth in [3usize, 6] {
        let canonical: String = (1..=depth).map(|l| format!("/l{l}")).collect();
        let localized: String = (1..=depth).map(|l| format!("/f{l}")).collect();
        let found = source
            .resolve(scope, "fr", &localized, None)
            .unwrap()
            .expect("resolves");
        assert_eq!(found.served_by, LocalizedServedBy::Index, "index not Ready");
        assert_eq!(found.canonical_path, canonical);

        let params = json!({ "depth": depth, "siblings": 20 });
        let st_local = pct(&format!("storage_localized_{depth}"), || async {
            let found = source
                .resolve(scope, "fr", &localized, None)
                .unwrap()
                .unwrap();
            let n = storage
                .nodes()
                .get(scope, &found.node_id, None)
                .await
                .unwrap();
            assert!(n.is_some());
        })
        .await;
        let st_path = pct(&format!("storage_path_{depth}"), || async {
            let n = storage.nodes().get_by_path(scope, &canonical, None).await;
            assert!(n.unwrap().is_some());
        })
        .await;
        record_pct(
            "L_storage_localized_resolve_get",
            params.clone(),
            st_local,
            None,
        );
        record_pct("L_storage_get_by_path", params.clone(), st_path, None);

        let cases = [
            (
                "sql_localized_star",
                format!("SELECT * FROM '{WS}' WHERE locale = 'fr' AND __localized_path = '{localized}'"),
                st_local,
            ),
            (
                "sql_localized_id",
                format!("SELECT id FROM '{WS}' WHERE locale = 'fr' AND __localized_path = '{localized}'"),
                st_local,
            ),
            (
                "sql_resolve_path",
                // A scalar SELECT needs a FROM; the row is the root page.
                format!(
                    "SELECT RESOLVE_PATH('{WS}', 'fr', '{localized}') AS id FROM '{WS}' \
                     WHERE path = '/l1'"
                ),
                st_local,
            ),
            (
                "sql_canonical_fr_star",
                format!("SELECT * FROM '{WS}' WHERE locale = 'fr' AND path = '{canonical}'"),
                st_path,
            ),
        ];
        for (name, sql, baseline) in cases {
            assert_eq!(run(&engine, &sql).await, 1, "{sql}");
            let took = pct(&format!("{name}_{depth}"), || async {
                run(&engine, &sql).await;
            })
            .await;
            record_pct(&format!("L_{name}"), params.clone(), took, Some(baseline));
        }
    }
}
