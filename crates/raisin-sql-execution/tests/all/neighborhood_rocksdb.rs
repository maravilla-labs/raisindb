// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `NEIGHBORS(..., depth => n)` and the pinned-endpoint `GRAPH_TABLE` reads,
//! executed against a REAL RocksDB through the SQL surface.
//!
//! The unit tests in `graph_neighborhood` and `pgq::matching::pushdown_tests`
//! prove the algorithms against an instrumented in-memory graph. What only a
//! real engine can show is the wiring: the argument grammar reaching the
//! planner, the per-node indexes returning what the global one holds, revisions
//! and tombstones, row-level security, and the recommended query shapes
//! actually running as SQL.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, RelationRef};
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, Row, StaticCatalog};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RelationRepository, RepoScope, Storage,
    StorageScope, WorkspaceRepository,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_nbh";
const REPO: &str = "r_nbh";
const BRANCH: &str = "main";
const WS: &str = "social";
const NODE_TYPE: &str = "test:Person";

type Store = Arc<raisin_rocksdb::RocksDBStorage>;
type Engine = QueryEngine<raisin_rocksdb::RocksDBStorage>;

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

async fn storage() -> (Store, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(tmp.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await;
    // A real repository lists its workspaces; the pinned reads and the
    // neighbourhood start lookup ask that list where an id lives.
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("register workspace");
    (Arc::new(storage), tmp)
}

fn engine(storage: &Store, auth: Option<AuthContext>) -> Engine {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog));
    match auth {
        Some(a) => engine.with_auth(a),
        None => engine.with_auth(AuthContext::system()),
    }
}

async fn create(storage: &Store, id: &str, owner: &str) {
    let mut props = HashMap::new();
    props.insert("name".to_string(), PropertyValue::String(id.to_string()));
    props.insert(
        "owner".to_string(),
        PropertyValue::String(owner.to_string()),
    );
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

async fn link(storage: &Store, from: &str, to: &str, ty: &str) {
    let rel = RelationRef::new(
        to.to_string(),
        WS.to_string(),
        NODE_TYPE.to_string(),
        ty.to_string(),
        None,
    );
    storage
        .relations()
        .add_relation(scope(), from, NODE_TYPE, rel)
        .await
        .unwrap_or_else(|e| panic!("relation {from}->{to}: {e}"));
}

async fn unlink(storage: &Store, from: &str, to: &str, ty: &str) {
    storage
        .relations()
        .remove_relation(scope(), from, WS, to, Some(ty))
        .await
        .unwrap_or_else(|e| panic!("unrelate {from}->{to}: {e}"));
}

async fn build(storage: &Store, nodes: &[&str], edges: &[(&str, &str, &str)]) {
    for n in nodes {
        create(storage, n, "bob").await;
    }
    for (a, b, t) in edges {
        link(storage, a, b, t).await;
    }
}

async fn run(engine: &Engine, sql: &str) -> Result<Vec<Row>, String> {
    let mut stream = engine
        .execute(sql)
        .await
        .map_err(|e| format!("execute failed: {e}"))?;
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        out.push(row.map_err(|e| format!("row failed: {e}"))?);
    }
    Ok(out)
}

async fn ok(engine: &Engine, sql: &str) -> Vec<Row> {
    run(engine, sql)
        .await
        .unwrap_or_else(|e| panic!("{e}\nSQL: {sql}"))
}

fn text(row: &Row, name: &str) -> String {
    match row.get(name) {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!(
            "column {name}: expected text, got {other:?} in {:?}",
            row.columns
        ),
    }
}

fn int(row: &Row, name: &str) -> i64 {
    match row.get(name) {
        Some(PropertyValue::Integer(i)) => *i,
        other => panic!(
            "column {name}: expected integer, got {other:?} in {:?}",
            row.columns
        ),
    }
}

/// `(id, depth, via_count)` triples from a set-form NEIGHBORS query.
fn triples(rows: &[Row]) -> Vec<(String, i64, i64)> {
    let mut v: Vec<_> = rows
        .iter()
        .map(|r| (text(r, "id"), int(r, "depth"), int(r, "via_count")))
        .collect();
    v.sort();
    v
}

fn t(id: &str, d: i64, v: i64) -> (String, i64, i64) {
    (id.to_string(), d, v)
}

/// me knows x, y, z. x and y know t. z knows u. x knows y (so y is a direct
/// friend AND a friend of a friend).
async fn social(storage: &Store) {
    build(
        storage,
        &["me", "x", "y", "z", "t", "u", "far"],
        &[
            ("me", "x", "KNOWS"),
            ("me", "y", "KNOWS"),
            ("me", "z", "KNOWS"),
            ("x", "t", "KNOWS"),
            ("y", "t", "KNOWS"),
            ("z", "u", "KNOWS"),
            ("x", "y", "KNOWS"),
            ("u", "far", "KNOWS"),
            ("me", "x", "BLOCKS"),
        ],
    )
    .await;
}

// ---------------------------------------------------------------------------
// The set form
// ---------------------------------------------------------------------------

#[tokio::test]
async fn friends_of_friends_with_mutual_counts() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    let rows = ok(
        &engine,
        "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', depth => 2, min_depth => 2, \
         types => 'KNOWS')",
    )
    .await;
    // t is reached through x and y (2 mutual connections), u through z (1).
    // y is a direct friend, so it is depth 1 and NOT a friend of a friend.
    assert_eq!(triples(&rows), vec![t("t", 2, 2), t("u", 2, 1)]);
}

#[tokio::test]
async fn each_node_is_reported_once_with_its_shortest_distance() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    let rows = ok(
        &engine,
        "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', depth => 3)",
    )
    .await;
    assert_eq!(
        triples(&rows),
        vec![
            t("far", 3, 1),
            t("t", 2, 2),
            t("u", 2, 1),
            t("x", 1, 1),
            t("y", 1, 1),
            t("z", 1, 1),
        ],
        "x appears once although two relation types join me and x; y is depth 1 \
         although x also reaches it"
    );
}

#[tokio::test]
async fn the_third_positional_argument_is_still_the_relation_type() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    let rows = ok(
        &engine,
        "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', 'BLOCKS', depth => 2)",
    )
    .await;
    assert_eq!(triples(&rows), vec![t("x", 1, 1)]);
}

#[tokio::test]
async fn in_and_both_follow_relations_backwards() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    let rows = ok(
        &engine,
        "SELECT id, depth, via_count FROM NEIGHBORS('t', 'IN', depth => 2, types => 'KNOWS')",
    )
    .await;
    assert_eq!(
        triples(&rows),
        vec![t("me", 2, 2), t("x", 1, 1), t("y", 1, 1)]
    );

    let both = ok(
        &engine,
        "SELECT id FROM NEIGHBORS('t', 'BOTH', depth => 1, types => 'KNOWS')",
    )
    .await;
    assert_eq!(both.len(), 2);
}

#[tokio::test]
async fn the_original_form_is_unchanged() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    // One row per RELATION, with its type: x is reached by KNOWS and BLOCKS.
    let rows = ok(
        &engine,
        "SELECT id, relation_type FROM NEIGHBORS('me', 'OUT', NULL)",
    )
    .await;
    let mut got: Vec<(String, String)> = rows
        .iter()
        .map(|r| (text(r, "id"), text(r, "relation_type")))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("x".into(), "BLOCKS".into()),
            ("x".into(), "KNOWS".into()),
            ("y".into(), "KNOWS".into()),
            ("z".into(), "KNOWS".into()),
        ]
    );
}

/// The one-hop form falls back to the global index only for a start that is NOT
/// in the query's workspace; a node that is here and has no relations is empty.
#[tokio::test]
async fn the_one_hop_form_still_finds_a_node_in_another_workspace() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    create(&storage, "loner", "bob").await;
    let engine = engine(&storage, None);

    for start in ["me", "social:/me"] {
        let rows = ok(
            &engine,
            &format!("SELECT id FROM NEIGHBORS('{start}', 'OUT', 'KNOWS')"),
        )
        .await;
        assert_eq!(rows.len(), 3, "{start}");
    }
    let none = ok(&engine, "SELECT id FROM NEIGHBORS('loner', 'OUT', 'KNOWS')").await;
    assert!(none.is_empty());
    let none = ok(
        &engine,
        "SELECT id FROM NEIGHBORS('social:/loner', 'BOTH', NULL)",
    )
    .await;
    assert!(none.is_empty());
}

#[tokio::test]
async fn the_result_composes_with_ordinary_sql() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    // Membership: is `far` within three hops of me?
    let hit = ok(
        &engine,
        "SELECT depth FROM NEIGHBORS('me', 'OUT', depth => 3, types => 'KNOWS') WHERE id = 'far'",
    )
    .await;
    assert_eq!(int(&hit[0], "depth"), 3);
    let miss = ok(
        &engine,
        "SELECT depth FROM NEIGHBORS('me', 'OUT', depth => 2, types => 'KNOWS') WHERE id = 'far'",
    )
    .await;
    assert!(miss.is_empty());

    // Ordering and limiting are SQL's job.
    let top = ok(
        &engine,
        "SELECT id, via_count FROM NEIGHBORS('me', 'OUT', depth => 2, min_depth => 2) \
         ORDER BY via_count DESC, id LIMIT 1",
    )
    .await;
    assert_eq!(text(&top[0], "id"), "t");

    // Counting.
    let n = ok(
        &engine,
        "SELECT COUNT(*) AS n FROM NEIGHBORS('me', 'OUT', depth => 3, types => 'KNOWS')",
    )
    .await;
    assert_eq!(int(&n[0], "n"), 6);
}

#[tokio::test]
async fn mutual_connections_with_one_person_is_a_join_of_two_neighbourhoods() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    // Who do `me` and `t` both know? (t is known BY x and y, so look IN.)
    let rows = ok(
        &engine,
        "SELECT a.id FROM NEIGHBORS('me', 'OUT', depth => 1, types => 'KNOWS') a \
         JOIN NEIGHBORS('t', 'IN', depth => 1, types => 'KNOWS') b ON a.id = b.id \
         ORDER BY a.id",
    )
    .await;
    let ids: Vec<String> = rows.iter().map(|r| text(r, "id")).collect();
    assert_eq!(ids, vec!["x".to_string(), "y".to_string()]);
}

#[tokio::test]
async fn a_missing_start_or_a_start_without_relations_is_empty() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);
    for start in ["ghost", "far"] {
        let rows = ok(
            &engine,
            &format!("SELECT id FROM NEIGHBORS('{start}', 'OUT', depth => 3)"),
        )
        .await;
        assert!(rows.is_empty(), "{start}");
    }
}

#[tokio::test]
async fn a_path_names_the_start_too() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);
    let rows = ok(
        &engine,
        "SELECT id FROM NEIGHBORS('social:/me', 'OUT', depth => 1, types => 'KNOWS')",
    )
    .await;
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn bad_arguments_are_errors_that_say_what_is_valid() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    let cases = [
        (
            "NEIGHBORS('me', 'OUT', deep => 2)",
            "unknown argument 'deep'",
        ),
        (
            "NEIGHBORS('me', 'OUT', depth => 0)",
            "depth must be between",
        ),
        (
            "NEIGHBORS('me', 'OUT', depth => 99)",
            "depth must be between",
        ),
        (
            "NEIGHBORS('me', 'OUT', depth => 2, min_depth => 3)",
            "min_depth",
        ),
        (
            "NEIGHBORS('me', 'SIDEWAYS', depth => 2)",
            "direction must be",
        ),
        (
            "NEIGHBORS('me', 'OUT', depth => 2, depth => 3)",
            "given twice",
        ),
        ("NEIGHBORS('me', 'OUT', depth => 'two')", "integer literal"),
        (
            "NEIGHBORS('me', 'OUT', max_nodes => 0)",
            "max_nodes must be between",
        ),
    ];
    for (call, needle) in cases {
        let err = run(&engine, &format!("SELECT id FROM {call}"))
            .await
            .expect_err(call);
        assert!(err.contains(needle), "{call}: {err}");
    }
}

/// The budget is a refusal, never a short answer.
#[tokio::test]
async fn exceeding_the_visited_node_budget_fails_instead_of_truncating() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    // me + x, y, z, t, u, far = 7 visited at depth 3.
    let enough = ok(
        &engine,
        "SELECT id FROM NEIGHBORS('me', 'OUT', depth => 3, types => 'KNOWS', max_nodes => 7)",
    )
    .await;
    assert_eq!(enough.len(), 6);

    let err = run(
        &engine,
        "SELECT id FROM NEIGHBORS('me', 'OUT', depth => 3, types => 'KNOWS', max_nodes => 6)",
    )
    .await
    .expect_err("one node over budget must fail");
    assert!(
        err.contains("neighbourhood too large: narrow the query with depth, types or max_nodes"),
        "{err}"
    );
    assert!(
        !err.chars().any(|c| c.is_ascii_digit()),
        "no count may appear in the error: {err}"
    );
}

/// The planner takes up to 1,000,000; past the default only an admin is
/// served the larger budget (see `effective_max_nodes`), anyone else is held to
/// the default, which fails instead of truncating.
#[tokio::test]
async fn max_nodes_has_a_hard_maximum_and_an_ordinary_caller_still_runs() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    // `social` nodes belong to bob: an ordinary caller who reads all of them.
    let bob = engine(&storage, Some(owner_scoped_user("bob")));
    let rows = ok(
        &bob,
        "SELECT id FROM NEIGHBORS('me', 'OUT', depth => 3, types => 'KNOWS', max_nodes => 1000000)",
    )
    .await;
    assert_eq!(rows.len(), 6);
    let err = run(
        &bob,
        "SELECT id FROM NEIGHBORS('me', 'OUT', depth => 3, max_nodes => 1000001)",
    )
    .await
    .expect_err("past the maximum");
    assert!(
        err.contains("max_nodes must be between 1 and 1000000"),
        "{err}"
    );
}

#[tokio::test]
async fn a_removed_relation_is_gone_from_the_neighbourhood() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);
    unlink(&storage, "x", "t", "KNOWS").await;

    let rows = ok(
        &engine,
        "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', depth => 2, min_depth => 2, \
         types => 'KNOWS')",
    )
    .await;
    assert_eq!(triples(&rows), vec![t("t", 2, 1), t("u", 2, 1)]);
}

// ---------------------------------------------------------------------------
// Row-level security
// ---------------------------------------------------------------------------

fn owner_scoped_user(user: &str) -> AuthContext {
    AuthContext::for_user(user).with_permissions(ResolvedPermissions {
        user_id: user.to_string(),
        email: Some(format!("{user}@test.com")),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new("/**", vec![Operation::Read])
            .with_condition("node.owner == auth.user_id".to_string())],
        is_system_admin: false,
        home: None,
        resolved_at: Some(std::time::Instant::now()),
    })
}

/// `me` -> `mid` (owned by bob) -> `end` (owned by alice), `me` -> `own`.
async fn owned(storage: &Store) {
    create(storage, "me", "alice").await;
    create(storage, "mid", "bob").await;
    create(storage, "end", "alice").await;
    create(storage, "own", "alice").await;
    link(storage, "me", "mid", "KNOWS").await;
    link(storage, "mid", "end", "KNOWS").await;
    link(storage, "me", "own", "KNOWS").await;
}

#[tokio::test]
async fn nodes_the_caller_cannot_read_are_left_out() {
    let (storage, _tmp) = storage().await;
    owned(&storage).await;

    let system = engine(&storage, None);
    let all = ok(
        &system,
        "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', depth => 2)",
    )
    .await;
    assert_eq!(
        triples(&all),
        vec![t("end", 2, 1), t("mid", 1, 1), t("own", 1, 1)]
    );

    // alice may read only what she owns: `mid` (bob's) is not reported - the
    // same rule the one-hop form applies - but it is still walked through, so
    // `end` is found. That is the endpoints-only policy GRAPH_TABLE documents.
    // `end`'s only predecessor is `mid`, which alice cannot read, so its
    // via_count is 0: hidden nodes are never counted.
    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    let seen = ok(
        &alice,
        "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', depth => 2)",
    )
    .await;
    assert_eq!(triples(&seen), vec![t("end", 2, 0), t("own", 1, 1)]);

    // And the one-hop form agrees on what a hop-1 caller sees.
    let one_hop = ok(&alice, "SELECT id FROM NEIGHBORS('me', 'OUT', NULL)").await;
    let ids: BTreeSet<String> = one_hop.iter().map(|r| text(r, "id")).collect();
    assert_eq!(ids, BTreeSet::from(["own".to_string()]));

    // A caller who owns nothing sees nothing.
    let carol = engine(&storage, Some(owner_scoped_user("carol")));
    let none = ok(&carol, "SELECT id FROM NEIGHBORS('me', 'OUT', depth => 2)").await;
    assert!(none.is_empty());
}

/// A start the caller cannot read answers exactly like a start that does not
/// exist: no rows, no error, whatever lies behind it.
#[tokio::test]
async fn an_unreadable_start_is_indistinguishable_from_a_missing_one() {
    let (storage, _tmp) = storage().await;
    owned(&storage).await;
    // `mid` (bob's) leads to `end` (alice's): walking from it would find a node
    // alice may read.
    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    for start in ["mid", "/mid", "ghost", "/ghost"] {
        for shape in [
            "depth => 2",
            "depth => 2, min_depth => 2",
            "depth => 2, max_nodes => 1",
        ] {
            let rows = run(
                &alice,
                &format!("SELECT id, depth, via_count FROM NEIGHBORS('{start}', 'OUT', {shape})"),
            )
            .await;
            assert_eq!(rows.map(|r| r.len()), Ok(0), "{start} {shape}");
        }
    }
    // The walk from `mid` does exist: the system caller is shown it.
    let system = engine(&storage, None);
    let rows = ok(
        &system,
        "SELECT id FROM NEIGHBORS('mid', 'OUT', depth => 2)",
    )
    .await;
    assert_eq!(rows.len(), 1);
}

/// `via_count` counts readable previous-level nodes only.
#[tokio::test]
async fn via_count_never_counts_a_node_the_caller_cannot_read() {
    let (storage, _tmp) = storage().await;
    owned(&storage).await;
    // A second, readable route to `end`, plus a hidden one.
    create(&storage, "mid2", "bob").await;
    link(&storage, "own", "end", "KNOWS").await;
    link(&storage, "me", "mid2", "KNOWS").await;
    link(&storage, "mid2", "end", "KNOWS").await;

    let q = "SELECT id, depth, via_count FROM NEIGHBORS('me', 'OUT', depth => 2, min_depth => 2)";
    let system = ok(&engine(&storage, None), q).await;
    assert_eq!(triples(&system), vec![t("end", 2, 3)]);
    let alice = ok(&engine(&storage, Some(owner_scoped_user("alice"))), q).await;
    assert_eq!(triples(&alice), vec![t("end", 2, 1)], "only `own` counts");
}

/// A GRAPH_TABLE pin on a node the caller cannot read and a pin on a node that
/// does not exist give the same rows (none), on both the one-hop and the
/// variable-length route.
#[tokio::test]
async fn a_graph_table_pin_on_a_hidden_node_answers_like_a_missing_one() {
    let (storage, _tmp) = storage().await;
    owned(&storage).await;
    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    for pattern in [
        "(a:Person)-[r:KNOWS]->(b:Person)",
        "(a:Person)-[r:KNOWS]->{1,3}(b:Person)",
        "ANY SHORTEST p = (a:Person)-[:KNOWS]->{1,3}(b:Person)",
    ] {
        let mut answers = Vec::new();
        for pin in ["mid", "ghost"] {
            let extra = if pattern.starts_with("ANY") {
                " AND b.id = 'end'"
            } else {
                ""
            };
            let rows = run(
                &alice,
                &format!(
                    "SELECT * FROM GRAPH_TABLE(MATCH {pattern} WHERE a.id = '{pin}'{extra} \
                     COLUMNS (a.id AS src, b.id AS dst))"
                ),
            )
            .await;
            answers.push(rows.map(|r| r.len()));
        }
        assert_eq!(answers, vec![Ok(0), Ok(0)], "{pattern}");
    }
}

/// An ordinary caller cannot choose `max_nodes`: neither a smaller nor a
/// larger value changes what it gets.
#[tokio::test]
async fn max_nodes_is_ignored_for_an_ordinary_caller() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let bob = engine(&storage, Some(owner_scoped_user("bob")));
    let q =
        |n: &str| format!("SELECT id FROM NEIGHBORS('me', 'OUT', depth => 3, types => 'KNOWS'{n})");
    let plain = ok(&bob, &q("")).await.len();
    assert_eq!(plain, 6);
    for n in [
        ", max_nodes => 1",
        ", max_nodes => 3",
        ", max_nodes => 1000000",
    ] {
        assert_eq!(ok(&bob, &q(n)).await.len(), plain, "{n}");
    }
    // The system caller can lower it, and is refused past it.
    let system = engine(&storage, None);
    assert!(run(&system, &q(", max_nodes => 3")).await.is_err());
}

// ---------------------------------------------------------------------------
// Against a brute-force oracle
// ---------------------------------------------------------------------------

struct Lcg(u64);
impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n.max(1)
    }
}

/// Distances by repeated relaxation over the plain edge list, written for
/// obviousness. `side`: 0 = OUT, 1 = IN, 2 = BOTH.
fn oracle(
    edges: &[(String, String, String)],
    start: &str,
    side: u8,
    types: &[&str],
    min_depth: i64,
    max_depth: i64,
) -> BTreeMap<String, (i64, i64)> {
    let follows = |t: &str| types.is_empty() || types.contains(&t);
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (a, b, ty) in edges {
        if !follows(ty) {
            continue;
        }
        if side != 1 {
            adj.entry(a).or_default().insert(b);
        }
        if side != 0 {
            adj.entry(b).or_default().insert(a);
        }
    }
    let mut dist: BTreeMap<&str, i64> = BTreeMap::from([(start, 0)]);
    for d in 0..max_depth {
        let layer: Vec<&str> = dist
            .iter()
            .filter(|(_, &x)| x == d)
            .map(|(k, _)| *k)
            .collect();
        for u in layer {
            for v in adj.get(u).into_iter().flatten() {
                dist.entry(v).or_insert(d + 1);
            }
        }
    }
    let mut out = BTreeMap::new();
    for (n, &d) in &dist {
        if d >= 1 && d >= min_depth {
            let via = dist
                .iter()
                .filter(|(u, &x)| x == d - 1 && adj.get(*u).is_some_and(|s| s.contains(n)))
                .count() as i64;
            out.insert(n.to_string(), (d, via));
        }
    }
    out
}

async fn random_case(seed: u64, n: usize, e: usize, undirected: bool) {
    let (storage, _tmp) = storage().await;
    let mut rng = Lcg(seed);
    let ids: Vec<String> = (0..n).map(|i| format!("n{i:03}")).collect();
    for id in &ids {
        create(&storage, id, "bob").await;
    }
    let tys = ["KNOWS", "FOLLOWS"];
    let mut edges: Vec<(String, String, String)> = Vec::new();
    let mut seen = HashSet::new();
    for _ in 0..e {
        let a = ids[rng.below(n as u64) as usize].clone();
        let b = ids[rng.below(n as u64) as usize].clone();
        let ty = tys[rng.below(2) as usize].to_string();
        if a == b || !seen.insert((a.clone(), b.clone(), ty.clone())) {
            continue;
        }
        link(&storage, &a, &b, &ty).await;
        edges.push((a.clone(), b.clone(), ty.clone()));
        if undirected && seen.insert((b.clone(), a.clone(), ty.clone())) {
            link(&storage, &b, &a, &ty).await;
            edges.push((b, a, ty));
        }
    }
    let engine = engine(&storage, None);

    for (dir, side) in [("OUT", 0u8), ("IN", 1), ("BOTH", 2)] {
        for (types_sql, types) in [
            ("", vec![]),
            (", types => 'KNOWS'", vec!["KNOWS"]),
            (", types => 'KNOWS,FOLLOWS'", vec!["KNOWS", "FOLLOWS"]),
        ] {
            for (min_d, max_d) in [(1, 1), (1, 3), (2, 3)] {
                for pick in 0..2u64 {
                    let from = &ids[((pick * 13 + seed) % n as u64) as usize];
                    let rows = ok(
                        &engine,
                        &format!(
                            "SELECT id, depth, via_count FROM NEIGHBORS('{from}', '{dir}', \
                             depth => {max_d}, min_depth => {min_d}{types_sql})"
                        ),
                    )
                    .await;
                    let got: BTreeMap<String, (i64, i64)> = triples(&rows)
                        .into_iter()
                        .map(|(i, d, v)| (i, (d, v)))
                        .collect();
                    assert_eq!(got.len(), rows.len(), "seed {seed}: a node came back twice");
                    let want = oracle(&edges, from, side, &types, min_d, max_d);
                    assert_eq!(
                        got, want,
                        "seed {seed} n={n} e={e} undirected={undirected} {dir} {types:?} \
                         depth {min_d}..{max_d} from {from}"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn matches_the_oracle_on_random_graphs_in_rocksdb() {
    for (seed, n, e) in [(21, 12, 20), (22, 40, 120), (23, 90, 200)] {
        for undirected in [false, true] {
            random_case(seed, n, e, undirected).await;
        }
    }
}

// ---------------------------------------------------------------------------
// GRAPH_TABLE: pinned endpoints through the SQL surface
// ---------------------------------------------------------------------------

/// A WHERE that cannot be pushed down (an OR) takes the global route; the same
/// question phrased with an AND-ed pin takes the per-node route. Both must agree.
#[tokio::test]
async fn a_pinned_graph_table_agrees_with_the_global_route() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    let shapes = [
        ("(a:Person)-[r:KNOWS]->(b:Person)", "a.id", "me"),
        ("(a:Person)<-[r:KNOWS]-(b:Person)", "a.id", "t"),
        ("(a:Person)-[r:KNOWS]->(b:Person)", "b.id", "t"),
        ("(a:Person)-[r:KNOWS|BLOCKS]->(b:Person)", "a.id", "me"),
        ("(a:Person)-[r:KNOWS]->{1,3}(b:Person)", "a.id", "me"),
        ("(a:Person)<-[r:KNOWS]-{1,2}(b:Person)", "a.id", "t"),
        (
            "(a:Person)-[r1:KNOWS]->(m:Person)-[r2:KNOWS]->(b:Person)",
            "a.id",
            "me",
        ),
    ];
    for (pattern, prop, value) in shapes {
        let pinned = format!(
            "SELECT * FROM GRAPH_TABLE(MATCH {pattern} WHERE {prop} = '{value}' \
             COLUMNS (a.id AS src, b.id AS dst))"
        );
        // `x OR x` is the same predicate, but the matcher does not look under OR.
        let unpinned = format!(
            "SELECT * FROM GRAPH_TABLE(MATCH {pattern} WHERE {prop} = '{value}' OR {prop} = '{value}' \
             COLUMNS (a.id AS src, b.id AS dst))"
        );
        let pairs = |rows: Vec<Row>| {
            let mut v: Vec<(String, String)> = rows
                .iter()
                .map(|r| (text(r, "src"), text(r, "dst")))
                .collect();
            v.sort();
            v
        };
        let a = pairs(ok(&engine, &pinned).await);
        let b = pairs(ok(&engine, &unpinned).await);
        assert!(!a.is_empty(), "{pattern}: fixture should match something");
        assert_eq!(a, b, "{pattern} where {prop} = {value}");
    }
}

#[tokio::test]
async fn pinning_by_path_works_and_agrees() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);
    let by_path = ok(
        &engine,
        "SELECT * FROM GRAPH_TABLE(MATCH (a:Person)-[r:KNOWS]->(b:Person) WHERE a.path = '/me' \
         COLUMNS (b.id AS dst))",
    )
    .await;
    let by_id = ok(
        &engine,
        "SELECT * FROM GRAPH_TABLE(MATCH (a:Person)-[r:KNOWS]->(b:Person) WHERE a.id = 'me' \
         COLUMNS (b.id AS dst))",
    )
    .await;
    assert_eq!(by_path.len(), 3);
    assert_eq!(by_id.len(), 3);
}

/// A relation removed at a later revision is still visible at the earlier one,
/// through the per-node route exactly as through the global one.
#[tokio::test]
async fn time_travel_agrees_between_the_two_routes() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);
    let before = ok(
        &engine,
        "SELECT * FROM GRAPH_TABLE(MATCH (a:Person)-[r:KNOWS]->(b:Person) WHERE a.id = 'me' \
         COLUMNS (b.id AS dst))",
    )
    .await;
    assert_eq!(before.len(), 3);
    unlink(&storage, "me", "z", "KNOWS").await;
    let after = ok(
        &engine,
        "SELECT * FROM GRAPH_TABLE(MATCH (a:Person)-[r:KNOWS]->(b:Person) WHERE a.id = 'me' \
         COLUMNS (b.id AS dst))",
    )
    .await;
    assert_eq!(
        after.len(),
        2,
        "the unrelated edge is gone from the per-node read"
    );
}

// ---------------------------------------------------------------------------
// Recommended shapes over GRAPH_TABLE output
// ---------------------------------------------------------------------------

#[tokio::test]
async fn count_and_group_by_work_over_a_pinned_two_hop_pattern() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);

    // Friends of friends with their mutual-connection count, the GRAPH_TABLE way
    // (one row per route, aggregated).
    let rows = ok(
        &engine,
        "SELECT fof, COUNT(*) AS mutual FROM GRAPH_TABLE(\
         MATCH (me:Person)-[r1:KNOWS]->(f:Person)-[r2:KNOWS]->(c:Person) \
         WHERE me.id = 'me' COLUMNS (f.id AS friend, c.id AS fof)) \
         GROUP BY fof ORDER BY fof",
    )
    .await;
    let got: Vec<(String, i64)> = rows
        .iter()
        .map(|r| (text(r, "fof"), int(r, "mutual")))
        .collect();
    // y is reached via x (x knows y) and is also a direct friend: GRAPH_TABLE
    // does not know that, which is exactly why the set form exists.
    assert_eq!(got, vec![("t".into(), 2), ("u".into(), 1), ("y".into(), 1)]);
}

#[tokio::test]
async fn shortest_path_between_two_pinned_nodes() {
    let (storage, _tmp) = storage().await;
    social(&storage).await;
    let engine = engine(&storage, None);
    let rows = ok(
        &engine,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:Person)-[:KNOWS]->{1,6}(b:Person) \
         WHERE a.id = 'me' AND b.id = 'far' COLUMNS (path_length(p) AS hops))",
    )
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(int(&rows[0], "hops"), 3);
}
