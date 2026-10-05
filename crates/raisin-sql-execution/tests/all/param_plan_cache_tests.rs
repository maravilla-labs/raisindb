//! Plan Phase 13d: the parameterized plan cache (`engine/prepared_params.rs`)
//! and the physical plan cache (`engine/physical_cache.rs`).
//!
//! The oracle for every row: the same statement with its parameters
//! substituted into the text and run through `execute` — what every
//! transport did before templates existed. Each test also proves, per call,
//! that the template (or the cached physical plan) actually answered.

use super::compound_index_hierarchy::{message_type, NODE_TYPE};
use super::localized_paths as lp;
use super::plan_cache_tests::{catalog, drain, engine, fixture, upsert_type, BRANCH, SERIAL, WS};
use raisin_models::auth::AuthContext;
use raisin_rocksdb::RocksDBStorage;
use raisin_sql_execution::{
    format_param_value, invalidate_compound_index_cache, substitute_params, template_cache_stats,
    ParamOutcome, QueryEngine,
};
use serde_json::{json, Value};

/// Rows of `sql` with `params` through the parameterized entry point, and how
/// the statement was prepared.
pub(super) async fn bound<S>(
    engine: &QueryEngine<S>,
    sql: &str,
    params: &[Value],
) -> (Vec<String>, ParamOutcome)
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    let (stream, outcome) = engine
        .execute_with_params_traced(sql, params, &format_param_value)
        .await
        .unwrap_or_else(|e| panic!("[{sql}] {params:?}: {e}"));
    (drain(stream, sql).await, outcome)
}

/// The oracle: the substituted text through `execute`.
async fn text<S>(engine: &QueryEngine<S>, sql: &str, params: &[Value]) -> Vec<String>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    let sql = substitute_params(sql, params).unwrap();
    drain(engine.execute(&sql).await.unwrap(), &sql).await
}

/// Every execution's rows equal the oracle's, and every execution after the
/// first is answered by the template.
async fn assert_binds<S>(engine: &QueryEngine<S>, sql: &str, runs: &[(Vec<Value>, usize)])
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    for (i, (params, expected)) in runs.iter().enumerate() {
        let (rows, outcome) = bound(engine, sql, params).await;
        assert_eq!(rows, text(engine, sql, params).await, "[{sql}] {params:?}");
        assert_eq!(rows.len(), *expected, "[{sql}] {params:?}: {rows:?}");
        if i > 0 {
            assert_eq!(outcome, ParamOutcome::Template, "[{sql}] {params:?}");
        }
    }
}

#[tokio::test]
async fn one_template_serves_every_value() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pp_values", "pp_values_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let cat = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &cat, AuthContext::system());
    let path = |p: &str| (vec![json!(p)], usize::from(p != "/nope"));
    assert_binds(
        &sys,
        &format!("SELECT id FROM '{WS}' WHERE path = $1"),
        &[
            path("/a/m0"),
            path("/a/m1"),
            path("/nope"),
            path("/a/m2"),
            path("/a"),
        ],
    )
    .await;
    assert_binds(
        &sys,
        &format!("SELECT * FROM '{WS}' WHERE id = $1"),
        &[
            (vec![json!("m1")], 1),
            (vec![json!("m2")], 1),
            (vec![json!("zz")], 0),
        ],
    )
    .await;
    assert_binds(
        &sys,
        &format!("SELECT id FROM '{WS}' WHERE CHILD_OF($1) ORDER BY path"),
        &[
            (vec![json!("/a")], 3),
            (vec![json!("/a/m0")], 0),
            (vec![json!("/a")], 3),
        ],
    )
    .await;
    assert_binds(
        &sys,
        &format!("SELECT id FROM '{WS}' WHERE id IN ($1, $2) ORDER BY id"),
        &[
            (vec![json!("m0"), json!("m2")], 2),
            (vec![json!("m1"), json!("zz")], 1),
        ],
    )
    .await;
}

#[tokio::test]
async fn a_locale_parameter_reads_its_own_locale() {
    let _serial = SERIAL.lock().await;
    let (storage, _dir, chair) = lp::setup_with(lp::config()).await;
    let engine = lp::engine(&storage);
    let name = format!(
        "SELECT __node_name FROM {} WHERE locale = $1 AND path = $2",
        lp::WS
    );
    // The base language has no translated name.
    for (locale, expected) in [("fr", "chaise"), ("en", "Null"), ("fr", "chaise")] {
        let params = [json!(locale), json!("/products/chair")];
        let (rows, _) = bound(&engine, &name, &params).await;
        assert_eq!(rows, text(&engine, &name, &params).await, "{locale}");
        assert!(rows[0].contains(expected), "{locale}: {rows:?}");
    }
    let (_, outcome) = bound(&engine, &name, &[json!("en"), json!("/products/chair")]).await;
    assert_eq!(outcome, ParamOutcome::Template);

    // The localized lookup operator, planned from the bound values.
    let lookup = format!(
        "SELECT id FROM {} WHERE locale = $1 AND __localized_path = $2",
        lp::WS
    );
    for (params, expected) in [
        ([json!("fr"), json!("/produits/chaise")], Some(&chair)),
        ([json!("fr"), json!("/produits/nope")], None),
        ([json!("fr"), json!("/produits/chaise")], Some(&chair)),
    ] {
        let (rows, _) = bound(&engine, &lookup, &params).await;
        assert_eq!(rows, text(&engine, &lookup, &params).await, "{params:?}");
        match expected {
            Some(id) => assert!(rows.len() == 1 && rows[0].contains(id.as_str()), "{rows:?}"),
            None => assert!(rows.is_empty(), "{rows:?}"),
        }
    }
}

#[tokio::test]
async fn compound_predicates_and_limit_bind_correctly() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pp_compound", "pp_compound_repo");
    let (storage, _tmp) = fixture(t, r).await;
    upsert_type(&storage, t, r, message_type()).await;
    build_compound(&storage, t, r).await;
    let cat = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &cat, AuthContext::system());

    let sql = format!(
        "SELECT name FROM '{WS}' WHERE CHILD_OF($1) AND node_type = $2 \
         ORDER BY created_at DESC LIMIT 2"
    );
    assert_binds(
        &sys,
        &sql,
        &[
            (vec![json!("/a"), json!(NODE_TYPE)], 2),
            (vec![json!("/a/m0"), json!(NODE_TYPE)], 0),
            (vec![json!("/a"), json!("test:Other")], 0),
            (vec![json!("/a"), json!(NODE_TYPE)], 2),
        ],
    )
    .await;
    let (plan, outcome, _) = sys
        .prepared_physical_plan_with_params(&sql, &[json!("/a"), json!(NODE_TYPE)])
        .await
        .unwrap();
    assert_eq!(outcome, ParamOutcome::Template);
    assert!(
        plan.contains("CompoundIndexScan"),
        "bound values reach the planner:\n{plan}"
    );

    // LIMIT's value steers the plan (pushdown): planned per value, and right.
    let limit = format!("SELECT id FROM '{WS}' WHERE CHILD_OF('/a') ORDER BY path LIMIT $1");
    for n in [1, 2, 3, 1] {
        let params = [json!(n)];
        let (rows, outcome) = bound(&sys, &limit, &params).await;
        assert_eq!(rows.len(), n as usize);
        assert_eq!(rows, text(&sys, &limit, &params).await);
        assert_ne!(outcome, ParamOutcome::Template, "LIMIT is value-dependent");
    }
}

pub(super) async fn build_compound(storage: &RocksDBStorage, t: &str, r: &str) {
    raisin_rocksdb::management::async_indexing::rebuild_indexes(
        storage,
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
}

#[tokio::test]
async fn the_batch_entry_point_shares_the_template() {
    let _serial = SERIAL.lock().await;
    let (t, r) = ("pp_batch", "pp_batch_repo");
    let (storage, _tmp) = fixture(t, r).await;
    let cat = catalog(&storage, t, r).await;
    let sys = engine(&storage, t, r, &cat, AuthContext::system());
    // The text pgwire's Execute, HTTP and WS hand over: `$n` and values.
    let sql = format!("SELECT id FROM '{WS}' WHERE path = $1 ");
    let (hits_before, _, _) = template_cache_stats();
    for path in ["/a/m0", "/a/m1", "/a/m2"] {
        let params = [json!(path)];
        let stream = sys
            .execute_batch_with_params(&sql, &params, &format_param_value)
            .await
            .unwrap();
        assert_eq!(drain(stream, &sql).await, text(&sys, &sql, &params).await);
    }
    // The first execution builds the template; the others are bound from it
    // (only these tests bind templates, and they run one at a time).
    assert!(
        template_cache_stats().0 >= hits_before + 2,
        "every execution after the first is answered by the one template"
    );
    // A placeholder the parameters do not fill fails exactly as the
    // substitution does.
    let err = sys
        .execute_batch_with_params(
            &format!("{sql} AND id = $2"),
            &[json!("/a")],
            &format_param_value,
        )
        .await
        .err()
        .expect("missing parameter");
    assert!(
        err.to_string().contains("Parameter $2 not provided"),
        "{err}"
    );
}
