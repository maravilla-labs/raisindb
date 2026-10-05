//! Plan Phase 13d: the template and physical plan caches let go of what
//! changed — a NodeType change, an index availability change, a different
//! caller, a schema change.

use super::compound_index_hierarchy::{message_type, NODE_TYPE};
use super::param_plan_cache_tests::{bound, build_compound};
use super::plan_cache_tests::{
    catalog, engine, fixture, reader, upsert_type, workspace, SERIAL, WS,
};
use raisin_models::auth::AuthContext;
use raisin_rocksdb::RocksDBStorage;
use raisin_sql_execution::{
    invalidate_compound_index_cache, invalidate_workspace_catalog, ParamOutcome, QueryEngine,
};
use serde_json::{json, Value};

/// The physical plan of a template-answered call, and whether it was reused.
async fn plan_of(
    engine: &QueryEngine<RocksDBStorage>,
    sql: &str,
    params: &[Value],
) -> (String, bool) {
    let (plan, outcome, reused) = engine
        .prepared_physical_plan_with_params(sql, params)
        .await
        .unwrap();
    assert_eq!(outcome, ParamOutcome::Template, "[{sql}]");
    (plan, reused)
}

#[tokio::test]
async fn the_template_and_physical_caches_invalidate() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pp_inval", "pp_inval_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let sql = format!(
        "SELECT name FROM '{WS}' WHERE CHILD_OF($1) AND node_type = $2 \
         ORDER BY created_at DESC LIMIT 3"
    );
    let params = [json!("/a"), json!(NODE_TYPE)];
    let cat = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &cat, AuthContext::system());
    let rows = bound(&sys, &sql, &params).await.0;
    assert_eq!(rows.len(), 3);
    let (first, _) = plan_of(&sys, &sql, &params).await;
    assert!(
        !first.contains("CompoundIndexScan"),
        "no index yet:\n{first}"
    );
    assert!(
        plan_of(&sys, &sql, &params).await.1,
        "the physical plan is reused"
    );

    // A NodeType change: the index is declared, not yet built. The physical
    // plan replans (new definitions) and asks for the index's availability.
    upsert_type(&storage, t, r, message_type()).await;
    invalidate_compound_index_cache();
    let (declared, reused) = plan_of(&sys, &sql, &params).await;
    assert!(!reused, "a NodeType change replans");
    assert!(
        !declared.contains("CompoundIndexScan"),
        "not built:\n{declared}"
    );
    assert!(plan_of(&sys, &sql, &params).await.1);

    // An index availability change: the build. Nothing else changed, so only
    // the recorded availability answer can tell the cached plan is stale.
    build_compound(&storage, t, r).await;
    let (built, reused) = plan_of(&sys, &sql, &params).await;
    assert!(!reused, "an availability change replans");
    assert!(built.contains("CompoundIndexScan"), "now used:\n{built}");
    assert_eq!(
        bound(&sys, &sql, &params).await.0,
        rows,
        "same rows via the index"
    );

    // A different caller: the same template and physical plan, its own RLS.
    let denied = engine(&storage, t, r, &cat, reader(false));
    let (denied_rows, outcome) = bound(&denied, &sql, &params).await;
    assert_eq!(outcome, ParamOutcome::Template);
    assert!(denied_rows.is_empty(), "leaked through a cached template");
    let allowed = engine(&storage, t, r, &cat, reader(true));
    assert_eq!(bound(&allowed, &sql, &params).await.0, rows);

    // A schema change: a new catalog, so a new template.
    workspace(&storage, t, r, "news").await;
    invalidate_workspace_catalog(t, r);
    let after = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &after, AuthContext::system());
    let (again, outcome) = bound(&sys, &sql, &params).await;
    assert_ne!(
        outcome,
        ParamOutcome::Template,
        "the old catalog's template is not used"
    );
    assert_eq!(again, rows);
    assert_eq!(bound(&sys, &sql, &params).await.1, ParamOutcome::Template);
}
