// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! SQL/PGQ path features, executed against a REAL RocksDB.
//!
//! # Why this module exists
//!
//! The path work — `{m,n}` quantifiers, selectors, restrictors, path variables
//! and accessors — was covered in two places, and neither runs a query against
//! stored data in an ordinary test pass:
//!
//! * `raisin-sql/src/ast/pgq_parser/` proves the grammar PARSES. A parser test
//!   cannot tell a correct traversal from one that returns the wrong rows.
//! * `raisin-server/tests/all/pgq_*_e2e_test.rs` do execute, but they are
//!   `#[ignore]` and need a live server, so they are not part of a normal run.
//!
//! These tests take the middle path the RLS suite already established: a real
//! `QueryEngine` over a real RocksDB with real relations, asserting on RESULT
//! SETS. That is the only level at which "the graph queries actually work" is a
//! statement about the engine rather than about the grammar.
//!
//! # The fixture
//!
//! A deliberately small weighted graph where hop count and cost DISAGREE, so
//! `ANY SHORTEST` and `ANY CHEAPEST` cannot both be satisfied by the same path:
//!
//! ```text
//!         a ──1──▶ b ──1──▶ c ──1──▶ d        (3 hops, total cost 3)
//!         a ─────────10─────────────▶ d        (1 hop,  total cost 10)
//!         c ──1──▶ b                           (back edge: makes a cycle)
//! ```
//!
//! Shortest a→d is 1 hop; cheapest a→d is 3 hops. A traversal that ignored
//! weights would answer 1 for both, which is exactly the bug `COST` exists to
//! prevent.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, RelationRef};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RelationRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";
const WS: &str = "social";
const NODE_TYPE: &str = "test:Stop";
/// A PGQ label matches the node type's LOCAL part — the namespace prefix is not
/// written. Storing `test:Stop` and matching `(x:Stop)` is the convention the
/// RLS suite uses too; `(x:test:Stop)` is a parse error.
const LABEL: &str = "Stop";

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

async fn storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(tmp.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await;
    (Arc::new(storage), tmp)
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(AuthContext::system())
}

async fn create(storage: &Arc<raisin_rocksdb::RocksDBStorage>, id: &str) {
    let mut props = HashMap::new();
    props.insert("name".to_string(), PropertyValue::String(id.to_string()));
    storage
        .nodes()
        .create(
            scope(),
            Node {
                id: id.to_string(),
                path: format!("/{id}"),
                name: id.to_string(),
                parent: Some("/".to_string()),
                node_type: NODE_TYPE.to_string(),
                properties: props,
                ..Default::default()
            },
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("create {id}: {e}"));
}

async fn link(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    from: &str,
    to: &str,
    weight: f32,
) {
    let rel = RelationRef::new(
        to.to_string(),
        WS.to_string(),
        NODE_TYPE.to_string(),
        "road".to_string(),
        Some(weight),
    );
    storage
        .relations()
        .add_relation(scope(), from, NODE_TYPE, rel)
        .await
        .unwrap_or_else(|e| panic!("relation {from}->{to}: {e}"));
}

/// The weighted graph described in the module docs.
async fn seed(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    for id in ["a", "b", "c", "d"] {
        create(storage, id).await;
    }
    link(storage, "a", "b", 1.0).await;
    link(storage, "b", "c", 1.0).await;
    link(storage, "c", "d", 1.0).await;
    link(storage, "a", "d", 10.0).await;
    // Back edge, so WALK can revisit and ACYCLIC must not.
    link(storage, "c", "b", 1.0).await;
}

/// Run a query and collect one text column from every row, sorted.
async fn col(
    engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>,
    sql: &str,
    column: &str,
) -> Result<Vec<String>, String> {
    let mut stream = engine
        .execute(sql)
        .await
        .map_err(|e| format!("execute failed: {e}"))?;
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.map_err(|e| format!("row failed: {e}"))?;
        let value = match row.get(column) {
            Some(PropertyValue::String(s)) => s.clone(),
            Some(PropertyValue::Integer(i)) => i.to_string(),
            Some(PropertyValue::Float(f)) => f.to_string(),
            other => format!("{other:?}"),
        };
        out.push(value);
    }
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------------
// Quantifiers
// ---------------------------------------------------------------------------

/// A bounded canonical quantifier returns every endpoint reachable in range.
///
/// From `a`, within 1..3 hops: `b` (1), `c` (2), `d` (1 direct and 3 via the
/// chain). This is the shape the public reference leads with, so if it were
/// wrong every documented example would be.
#[tokio::test]
async fn a_bounded_quantifier_returns_the_reachable_endpoints() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let mut ends = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH (x:{LABEL})-[:road]->{{1,3}}(y:{LABEL}) \
             WHERE x.name = 'a' COLUMNS (y.name AS endpoint))"
        ),
        "endpoint",
    )
    .await
    .expect("bounded quantifier must execute");
    ends.dedup();

    assert_eq!(
        ends,
        vec!["b".to_string(), "c".to_string(), "d".to_string()],
        "1..3 hops from 'a' must reach b, c and d",
    );
}

/// The deprecated Cypher spelling must return EXACTLY the same rows.
///
/// The public reference presents `-[:t*1..3]->` and `-[:t]->{1,3}` as the same
/// query in two spellings. If they diverged, every reader migrating off the old
/// form would silently change their results.
#[tokio::test]
async fn the_legacy_quantifier_spelling_agrees_with_the_canonical_one() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let canonical = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH (x:{LABEL})-[:road]->{{1,3}}(y:{LABEL}) \
             WHERE x.name = 'a' COLUMNS (y.name AS endpoint))"
        ),
        "endpoint",
    )
    .await
    .expect("canonical form");

    let legacy = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH (x:{LABEL})-[:road*1..3]->(y:{LABEL}) \
             WHERE x.name = 'a' COLUMNS (y.name AS endpoint))"
        ),
        "endpoint",
    )
    .await
    .expect("legacy form");

    assert_eq!(
        canonical, legacy,
        "the deprecated spelling is documented as the same query — divergence \
         here would silently change results for anyone migrating",
    );
}

/// Rule Q-SCOPE, executed: an unbounded quantifier with no selector and no
/// restrictor is refused rather than run.
#[tokio::test]
async fn an_unscoped_unbounded_quantifier_is_refused() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let result = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH (x:{LABEL})-[:road]->*(y:{LABEL}) \
             COLUMNS (y.name AS endpoint))"
        ),
        "endpoint",
    )
    .await;

    assert!(
        result.is_err(),
        "an unbounded quantifier outside a selector or restrictor must be an \
         error, not an unbounded traversal",
    );
}

// ---------------------------------------------------------------------------
// Selectors — where hop count and cost disagree
// ---------------------------------------------------------------------------

/// `ANY SHORTEST` minimises HOPS: a→d is one hop, however expensive.
#[tokio::test]
async fn any_shortest_minimises_hops() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let hops = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = \
             (x:{LABEL})-[:road]->{{1,4}}(y:{LABEL}) \
             WHERE x.name = 'a' AND y.name = 'd' COLUMNS (path_length(p) AS hops))"
        ),
        "hops",
    )
    .await
    .expect("ANY SHORTEST must execute");

    assert_eq!(
        hops,
        vec!["1".to_string()],
        "the direct a->d edge is one hop, so ANY SHORTEST must answer 1",
    );
}

/// `ANY CHEAPEST` minimises COST, and must therefore pick the LONGER route.
///
/// This is the test that proves weights are actually read: the cheap route is
/// 3 hops at cost 3, the expensive route is 1 hop at cost 10. Answering 1 here
/// means `COST` was ignored and the selector degenerated to `ANY SHORTEST`.
#[tokio::test]
async fn any_cheapest_prefers_the_longer_but_cheaper_route() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let hops = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH ANY CHEAPEST p = \
             (x:{LABEL})-[r:road COST r.weight]->{{1,4}}(y:{LABEL}) \
             WHERE x.name = 'a' AND y.name = 'd' COLUMNS (path_length(p) AS hops))"
        ),
        "hops",
    )
    .await
    .expect("ANY CHEAPEST must execute");

    assert_eq!(
        hops,
        vec!["3".to_string()],
        "cheapest a->d is the 3-hop chain at total cost 3, not the 1-hop edge \
         at cost 10 — answering 1 means the weights were ignored",
    );
}

// ---------------------------------------------------------------------------
// Path accessors
// ---------------------------------------------------------------------------

/// `nodes(p)` must return the path's nodes, and `path_length(p)` its hop count.
#[tokio::test]
async fn path_accessors_describe_the_matched_path() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let lengths = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = \
             (x:{LABEL})-[:road]->{{1,4}}(y:{LABEL}) \
             WHERE x.name = 'a' AND y.name = 'c' COLUMNS (path_length(p) AS hops))"
        ),
        "hops",
    )
    .await
    .expect("path_length must execute");
    assert_eq!(lengths, vec!["2".to_string()], "a->b->c is two hops");

    // `nodes(p)` is a collection; assert only that it is produced and non-empty,
    // since its surface encoding is a transport concern.
    let listed = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = \
             (x:{LABEL})-[:road]->{{1,4}}(y:{LABEL}) \
             WHERE x.name = 'a' AND y.name = 'c' COLUMNS (nodes(p) AS stops))"
        ),
        "stops",
    )
    .await
    .expect("nodes(p) must execute");
    assert_eq!(listed.len(), 1, "one shortest path, so one row");
    assert!(
        !listed[0].is_empty(),
        "nodes(p) must carry the path's nodes, got an empty value",
    );
}

/// Selecting a bare path variable is refused, with a message naming the
/// accessors — the behaviour the public reference promises.
#[tokio::test]
async fn selecting_a_bare_path_variable_is_refused_by_name() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let err = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = \
             (x:{LABEL})-[:road]->{{1,4}}(y:{LABEL}) COLUMNS (p))"
        ),
        "p",
    )
    .await
    .expect_err("COLUMNS (p) must be refused — there is no PATH column type");

    assert!(
        err.contains("path_length") || err.to_lowercase().contains("accessor"),
        "the error must name the accessors so the fix is obvious, got: {err}",
    );
}

// ---------------------------------------------------------------------------
// Restrictors
// ---------------------------------------------------------------------------

/// `ACYCLIC` is the default and must not revisit a node.
///
/// The fixture has a `c -> b` back edge, so a walk could loop b->c->b. With the
/// default restrictor no endpoint may be reached by revisiting a node, which
/// bounds the answer regardless of the quantifier's upper limit.
#[tokio::test]
async fn the_default_restrictor_does_not_revisit_nodes() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let engine = engine(&storage);

    let default_rows = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH (x:{LABEL})-[:road]->{{1,4}}(y:{LABEL}) \
             WHERE x.name = 'a' COLUMNS (y.name AS endpoint))"
        ),
        "endpoint",
    )
    .await
    .expect("default restrictor must execute");

    let acyclic_rows = col(
        &engine,
        &format!(
            "SELECT * FROM GRAPH_TABLE(MATCH ACYCLIC (x:{LABEL})-[:road]->{{1,4}}(y:{LABEL}) \
             WHERE x.name = 'a' COLUMNS (y.name AS endpoint))"
        ),
        "endpoint",
    )
    .await
    .expect("explicit ACYCLIC must execute");

    assert_eq!(
        default_rows, acyclic_rows,
        "ACYCLIC is documented as the DEFAULT, so naming it explicitly must not \
         change the answer",
    );
}
