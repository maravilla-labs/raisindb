//! Plan Phase 13d: a SQL point read does not pay the `has_children` probe —
//! no SQL column shows it — while every API read (`get`, `get_by_path`) still
//! carries it, as decision D4 requires. `get_for_read` is otherwise exactly
//! the node `get` / `get_by_path` returns.

use super::plan_cache_tests::{catalog, drain, engine, fixture, BRANCH, WS};
use raisin_models::auth::AuthContext;
use raisin_storage::{
    BranchRepository, NodeLocator, NodeRepository, ReadOpts, Storage, StorageScope,
};

#[tokio::test]
async fn sql_reads_skip_the_child_probe_and_api_reads_keep_it() {
    let (t, r) = ("pr_shape", "pr_shape_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let scope = StorageScope::new(t, r, BRANCH, WS);
    let head = storage
        .branches()
        .get_branch(t, r, BRANCH)
        .await
        .unwrap()
        .unwrap()
        .head;

    for (id, path, has_children) in [("a", "/a", true), ("m0", "/a/m0", false)] {
        for at in [None, Some(&head)] {
            let api = storage.nodes().get(scope, id, at).await.unwrap().unwrap();
            assert_eq!(api.has_children, Some(has_children), "get {id}");
            let api_path = storage
                .nodes()
                .get_by_path(scope, path, at)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                api_path.has_children,
                Some(has_children),
                "get_by_path {path}"
            );

            for locator in [NodeLocator::Id(id.into()), NodeLocator::Path(path.into())] {
                let row = storage
                    .nodes()
                    .get_for_read(scope, &locator, at, &ReadOpts::default())
                    .await
                    .unwrap()
                    .unwrap();
                let mut expected = api.clone();
                expected.has_children = None;
                assert_eq!(row, expected, "{locator:?} at {at:?}");

                let probed = ReadOpts {
                    has_children: true,
                    ..ReadOpts::default()
                };
                let with = storage
                    .nodes()
                    .get_for_read(scope, &locator, at, &probed)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(with, api, "{locator:?} asked for the probe");
            }
        }
    }

    // The SQL row never had the column.
    let cat = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &cat, AuthContext::system());
    let mut stream = sys
        .execute(&format!("SELECT * FROM '{WS}' WHERE path = '/a'"))
        .await
        .unwrap();
    let row = futures::StreamExt::next(&mut stream)
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.columns.keys().all(|k| !k.contains("has_children")),
        "{:?}",
        row.columns.keys()
    );
    assert!(drain(stream, "").await.is_empty());
}
