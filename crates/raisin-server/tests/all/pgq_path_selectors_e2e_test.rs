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

//! **SQL/PGQ path support, proven against a real server over data written in SQL.**
//!
//! Every node and every relation in the fixture is written with `INSERT` and
//! `RELATE` — no REST shortcut — and every assertion reads back through a SQL
//! transport. `pgq_path_e2e_test` already covers the accessor shapes and the
//! TRAIL/WALK restrictors; this module covers what it does not:
//!
//! * the **ordered intermediate nodes** of a route, not just its endpoints or
//!   its length — the entire point of a path variable;
//! * **`ALL SHORTEST`** returning *every* minimum-hop route, where several tie;
//! * **`ANY`** returning exactly one arbitrary route;
//! * **`ANY CHEAPEST`** on a fixture where the cheap route is *longer* than the
//!   short route (5 hops at cost 5 vs 2 hops at cost 20), so a hop-count
//!   implementation returns the wrong nodes, not merely the wrong number;
//! * every **quantifier form** — `{m}`, `{m,n}`, `{m,}`, `*`, `+`, `?` — plus
//!   the deprecated Cypher-style `*m..n` behaving as the documented alias;
//! * **rule Q-SCOPE**: an unbounded quantifier with neither selector nor
//!   restrictor is a parse error naming both remedies;
//! * the same route crossing **HTTP, WebSocket and PGWire** intact.
//!
//! # The fixture
//!
//! ```text
//!            link w=10        link w=10
//!      s ------------- x ------------- t
//!      |                               ^
//!      |  link w=10      link w=10     |
//!      +-------------- y --------------+
//!      |                               |
//!      | w=1     w=1     w=1     w=1   |
//!      +-- c1 --- c2 --- c3 --- c4 ----+
//!
//!      s -[alt_a w=1]-> m1     s -[alt_b w=1]-> m2
//!      r1 -[ring w=1]-> r2 -[ring w=1]-> r1
//! ```
//!
//! `s`→`t` therefore has **two** distinct 2-hop routes costing 20 (so
//! `ALL SHORTEST` has something to return more than one of) and **one** 5-hop
//! route costing 5. Fewest hops and lowest cost are different routes *through
//! different nodes*, which is the only construction where a wrong answer is
//! visible in the node sequence rather than only in a number.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{
    admin_user_id, bootstrap_admin, create_api_key, grant_pgwire_identity, http_post, http_put,
    pg_error, pg_wait_ready, sql_http, sql_ws,
};
use serde_json::{json, Value};
use std::time::Duration;

const REPO: &str = "pgq_paths";
const BRANCH: &str = "main";
const WS: &str = "graph";
const NODE_TYPE: &str = "pgq:Hub";
const HTTP_PORT: u16 = 8109;
const PGWIRE_PORT: u16 = 55_434;

/// Every hub in the fixture.
const HUBS: [&str; 11] = ["s", "x", "y", "t", "c1", "c2", "c3", "c4", "m1", "m2", "r1"];

/// `(source, target, relation_type, weight)`.
const EDGES: [(&str, &str, &str, f64); 14] = [
    // Two 2-hop routes s->t, both costing 20.
    ("s", "x", "link", 10.0),
    ("x", "t", "link", 10.0),
    ("s", "y", "link", 10.0),
    ("y", "t", "link", 10.0),
    // One 5-hop route s->t costing 5.
    ("s", "c1", "link", 1.0),
    ("c1", "c2", "link", 1.0),
    ("c2", "c3", "link", 1.0),
    ("c3", "c4", "link", 1.0),
    ("c4", "t", "link", 1.0),
    // A two-type alternation off s.
    ("s", "m1", "alt_a", 1.0),
    ("s", "m2", "alt_b", 1.0),
    // A 2-cycle, so is_acyclic/is_trail have something to disagree about.
    ("r1", "r2", "ring", 1.0),
    ("r2", "r1", "ring", 1.0),
    // An edge into the ring from outside it, so a `ring` traversal has more
    // than one possible start node.
    ("s", "r1", "ring", 1.0),
];

// ------------------------------------------------------------------ plumbing

async fn sql(base: &str, token: &str, query: &str) -> Result<Value, String> {
    sql_http(base, token, REPO, query).await
}

/// Rows of a SQL response with any `graph_table.` qualifier stripped.
fn rows(response: &Value) -> Vec<Value> {
    response["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows in response: {response}"))
        .iter()
        .map(|row| match row.as_object() {
            Some(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.rsplit('.').next().unwrap_or(k).to_string(), v.clone()))
                    .collect(),
            ),
            None => row.clone(),
        })
        .collect()
}

fn hops(row: &Value) -> i64 {
    row["hops"]
        .as_i64()
        .unwrap_or_else(|| panic!("no integer `hops` in {row}"))
}

/// Node ids of a `nodes(p)` column, in path order.
fn node_ids(row: &Value, column: &str) -> Vec<String> {
    row[column]
        .as_array()
        .unwrap_or_else(|| panic!("`{column}` is not an array in {row}"))
        .iter()
        .map(|n| {
            n["id"]
                .as_str()
                .unwrap_or_else(|| panic!("node identity without an id: {n}"))
                .to_string()
        })
        .collect()
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

// ---------------------------------------------------------------- provisioning

async fn provision(base: &str, token: &str) {
    http_post(
        base,
        "/api/repositories",
        token,
        json!({ "repo_id": REPO, "description": "PGQ path selectors", "default_branch": BRANCH }),
    )
    .await
    .expect("create repository");

    http_put(
        base,
        &format!("/api/workspaces/{REPO}/{WS}"),
        token,
        json!({
            "name": WS,
            "description": "Weighted hub graph written entirely in SQL",
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create workspace");

    http_post(
        base,
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        token,
        json!({
            "node_type": {
                "name": NODE_TYPE,
                "description": "A hub in the path fixture",
                "properties": [{ "name": "title", "type": "String", "required": false }],
                "allowed_children": []
            },
            "commit": { "message": "Create pgq:Hub", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Write every node and every relation **through SQL**.
async fn seed(base: &str, token: &str) {
    for id in HUBS.iter().chain(["r2"].iter()) {
        sql(
            base,
            token,
            &format!(
                "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
                 ('{id}', '/{id}', '{NODE_TYPE}', '{{\"title\":\"{id}\"}}'::JSONB)"
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("INSERT {id} failed: {e}"));
    }

    for (src, tgt, rel, weight) in EDGES {
        sql(
            base,
            token,
            &format!(
                "RELATE FROM path='/{src}' IN WORKSPACE '{WS}' \
                 TO path='/{tgt}' IN WORKSPACE '{WS}' TYPE '{rel}' WEIGHT {weight}"
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("RELATE {src}-{rel}->{tgt} failed: {e}"));
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
}

// ---------------------------------------------------------------------- test

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all pgq_path_selectors_e2e_test -- --ignored --nocapture
async fn pgq_path_selectors_end_to_end() {
    println!("\n=== SQL/PGQ path selectors, quantifiers and transports ===\n");

    let server = ServerHandle::start(ServerConfig::new(HTTP_PORT).with_pgwire(PGWIRE_PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    let base = server.base_url.clone();
    provision(&base, &token).await;
    seed(&base, &token).await;
    println!("[OK] fixture written entirely through SQL (INSERT + RELATE)");

    the_route_is_ordered_and_cheapest_is_not_shortest(&base, &token).await;
    any_shortest_returns_one_minimum_hop_route(&base, &token).await;
    all_shortest_returns_every_tied_route(&base, &token).await;
    bare_any_returns_exactly_one_route(&base, &token).await;
    quantifier_forms(&base, &token).await;
    legacy_cypher_quantifier_is_an_accepted_alias(&base, &token).await;
    unbounded_quantifier_needs_a_selector_or_restrictor(&base, &token).await;
    accessors_on_a_cycle(&base, &token).await;
    transport_parity(&base, &token).await;

    println!("\n=== SQL/PGQ path selectors: PASS ===\n");
}

// ---------------------------------------------------- 1. the ordered route

/// The whole point of the feature: the *sequence of nodes*, intermediates
/// included.
///
/// `ANY CHEAPEST` must return `s c1 c2 c3 c4 t` — five hops costing 5 — while
/// `ANY SHORTEST` returns a two-hop route costing 20. An implementation that
/// answers cheapest by hop count returns a *different set of nodes*, so this is
/// not a test two implementations could both pass.
async fn the_route_is_ordered_and_cheapest_is_not_shortest(base: &str, token: &str) {
    println!("--- ordered route, and CHEAPEST != SHORTEST ---");

    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY CHEAPEST p = (a:Hub)-[e:link COST e.weight]->{1,6}(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' \
           COLUMNS (path_length(p) AS hops, nodes(p) AS ns, edges(p) AS es, element_id(p) AS eid))",
    )
    .await
    .expect("ANY CHEAPEST query failed");

    let matched = rows(&response);
    assert_eq!(matched.len(), 1, "one cheapest route per pair: {matched:?}");
    let row = &matched[0];

    assert_eq!(
        node_ids(row, "ns"),
        vec!["s", "c1", "c2", "c3", "c4", "t"],
        "ANY CHEAPEST must return the ordered low-weight route through c1..c4; \
         answering with a 2-hop route means the weights were ignored: {row}"
    );
    assert_eq!(hops(row), 5, "{row}");

    let edges = row["es"].as_array().expect("edges(p) is an array");
    assert_eq!(edges.len(), 5, "edges(p) == path_length: {row}");
    let hop_pairs: Vec<(String, String)> = edges
        .iter()
        .map(|e| {
            (
                e["source_id"].as_str().unwrap_or_default().to_string(),
                e["target_id"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert_eq!(
        hop_pairs,
        vec![
            ("s".into(), "c1".into()),
            ("c1".into(), "c2".into()),
            ("c2".into(), "c3".into()),
            ("c3".into(), "c4".into()),
            ("c4".into(), "t".into()),
        ],
        "edges(p) must be in path order and consistent with nodes(p): {row}"
    );
    for edge in edges {
        assert_eq!(
            edge["weight"].as_f64(),
            Some(1.0),
            "each edge on the cheap route carries its weight: {edge}"
        );
    }

    let eid = row["eid"].as_str().expect("element_id is text");
    assert_eq!(
        eid, "graph:s|link|graph:c1|link|graph:c2|link|graph:c3|link|graph:c4|link|graph:t",
        "element_id must encode the whole ordered route"
    );

    println!("[PASS] cheapest = s c1 c2 c3 c4 t (5 hops, cost 5)");
}

// ------------------------------------------------------- 2. ANY SHORTEST

async fn any_shortest_returns_one_minimum_hop_route(base: &str, token: &str) {
    println!("--- ANY SHORTEST ---");
    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:Hub)-[e:link]->{1,6}(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' \
           COLUMNS (path_length(p) AS hops, nodes(p) AS ns))",
    )
    .await
    .expect("ANY SHORTEST query failed");

    let matched = rows(&response);
    assert_eq!(
        matched.len(),
        1,
        "ANY SHORTEST is one route per endpoint pair: {matched:?}"
    );
    let ids = node_ids(&matched[0], "ns");
    assert_eq!(hops(&matched[0]), 2, "{}", matched[0]);
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert_eq!(ids[0], "s");
    assert_eq!(ids[2], "t");
    assert!(
        ids[1] == "x" || ids[1] == "y",
        "the middle hop must be one of the two tied routes, got {ids:?}"
    );
    println!("[PASS] ANY SHORTEST = {ids:?}");
}

// ------------------------------------------------------- 3. ALL SHORTEST

/// Two routes tie at two hops. `ALL SHORTEST` must return **both**, and neither
/// of the 5-hop route's rows.
async fn all_shortest_returns_every_tied_route(base: &str, token: &str) {
    println!("--- ALL SHORTEST ---");
    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ALL SHORTEST p = (a:Hub)-[e:link]->{1,6}(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' \
           COLUMNS (path_length(p) AS hops, nodes(p) AS ns))",
    )
    .await
    .expect("ALL SHORTEST query failed");

    let matched = rows(&response);
    assert_eq!(
        matched.len(),
        2,
        "both tied 2-hop routes must come back — returning one is ANY SHORTEST, \
         returning three means the 5-hop route leaked in: {matched:?}"
    );

    let mut middles = Vec::new();
    for row in &matched {
        assert_eq!(hops(row), 2, "every ALL SHORTEST row is minimum-hop: {row}");
        let ids = node_ids(row, "ns");
        assert_eq!(ids[0], "s");
        assert_eq!(ids[2], "t");
        middles.push(ids[1].clone());
    }
    assert_eq!(
        sorted(middles.clone()),
        vec!["x".to_string(), "y".to_string()],
        "the two tied routes go through x and y: {middles:?}"
    );
    println!("[PASS] ALL SHORTEST returned both tied routes (via x and via y)");
}

// --------------------------------------------------------------- 4. ANY

async fn bare_any_returns_exactly_one_route(base: &str, token: &str) {
    println!("--- ANY ---");
    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY p = (a:Hub)-[e:link]->{1,6}(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' \
           COLUMNS (path_length(p) AS hops, nodes(p) AS ns))",
    )
    .await
    .expect("ANY query failed");

    let matched = rows(&response);
    assert_eq!(
        matched.len(),
        1,
        "ANY is one arbitrary route per endpoint pair: {matched:?}"
    );
    let ids = node_ids(&matched[0], "ns");
    assert_eq!(ids.first().map(String::as_str), Some("s"), "{ids:?}");
    assert_eq!(ids.last().map(String::as_str), Some("t"), "{ids:?}");
    println!("[PASS] ANY returned one route: {ids:?}");
}

// ------------------------------------------------------- 5. quantifiers

/// Every canonical quantifier form, read back as hop counts.
async fn quantifier_forms(base: &str, token: &str) {
    println!("--- quantifier forms ---");

    // `{2}` — exactly two hops from s: s-x-t, s-y-t, s-c1-c2.
    let exact = hop_multiset(base, token, "MATCH p = (a:Hub)-[e:link]->{2}(b:Hub)").await;
    assert_eq!(
        exact,
        vec![2, 2, 2],
        "`->{{2}}` must yield exactly-2-hop routes only: {exact:?}"
    );

    // `{1,2}` — the 1-hop and 2-hop routes.
    let ranged = hop_multiset(base, token, "MATCH p = (a:Hub)-[e:link]->{1,2}(b:Hub)").await;
    assert_eq!(
        ranged,
        vec![1, 1, 1, 2, 2, 2],
        "`->{{1,2}}` must yield 1- and 2-hop routes: {ranged:?}"
    );

    // `{5,}` — unbounded above, so it needs a restrictor (rule Q-SCOPE).
    let at_least = hop_multiset(base, token, "MATCH TRAIL p = (a:Hub)-[e:link]->{5,}(b:Hub)").await;
    assert_eq!(
        at_least,
        vec![5],
        "`->{{5,}}` under TRAIL must yield only the 5-hop route: {at_least:?}"
    );

    // `?` — {0,1}: the zero-hop path plus each single hop.
    let optional = hop_multiset(base, token, "MATCH p = (a:Hub)-[e:link]->?(b:Hub)").await;
    assert!(
        optional.contains(&0) && optional.contains(&1) && !optional.contains(&2),
        "`->?` is {{0,1}}: {optional:?}"
    );

    // `*` — {0,} under a selector.
    let star = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:Hub)-[e:link]->*(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect("`->*` under ANY SHORTEST failed");
    assert_eq!(rows(&star).len(), 1);
    assert_eq!(hops(&rows(&star)[0]), 2, "{star}");

    // `+` — {1,} under a selector.
    let plus = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:Hub)-[e:link]->+(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect("`->+` under ANY SHORTEST failed");
    assert_eq!(rows(&plus).len(), 1);
    assert_eq!(hops(&rows(&plus)[0]), 2, "{plus}");

    // An empty range is rejected rather than silently matching nothing.
    let err = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH p = (a:Hub)-[e:link]->{3,1}(b:Hub) \
           COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect_err("`{3,1}` must be rejected");
    assert!(
        err.to_lowercase().contains("quantifier"),
        "the error must name the quantifier, got: {err}"
    );

    println!("[PASS] {{m}}, {{m,n}}, {{m,}}, ?, *, + all behave as specified");
}

/// Hop counts of every route out of `s`, sorted — the shape most quantifier
/// assertions want.
async fn hop_multiset(base: &str, token: &str, match_clause: &str) -> Vec<i64> {
    let response = sql(
        base,
        token,
        &format!(
            "SELECT * FROM GRAPH_TABLE({match_clause} WHERE a.id = 's' \
               COLUMNS (path_length(p) AS hops))"
        ),
    )
    .await
    .unwrap_or_else(|e| panic!("{match_clause} failed: {e}"));
    sorted(rows(&response).iter().map(hops).collect())
}

// ----------------------------------------------- 6. the deprecated spelling

/// The Cypher-style `*m..n` written INSIDE the brackets is kept as a documented
/// compatibility alias. It must mean exactly what `->{m,n}` means.
///
/// It is also exempt from rule Q-SCOPE: bare `*` predates the rule and is capped
/// at `PathQuantifier::DEFAULT_MAX` instead, so it must still parse with neither
/// selector nor restrictor.
async fn legacy_cypher_quantifier_is_an_accepted_alias(base: &str, token: &str) {
    println!("--- deprecated Cypher-style quantifier ---");

    let legacy = hop_multiset(base, token, "MATCH p = (a:Hub)-[e:link*1..2]->(b:Hub)").await;
    let canonical = hop_multiset(base, token, "MATCH p = (a:Hub)-[e:link]->{1,2}(b:Hub)").await;
    assert_eq!(
        legacy, canonical,
        "`*1..2` is documented as an alias of `->{{1,2}}`; they disagree: \
         legacy={legacy:?} canonical={canonical:?}"
    );

    // Bare legacy `*` is unbounded but exempt from Q-SCOPE.
    let bare = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH p = (a:Hub)-[e:link*]->(b:Hub) \
           WHERE a.id = 's' AND b.id = 't' COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect(
        "bare legacy `*` is exempt from Q-SCOPE and must still parse with no \
         selector or restrictor",
    );
    let lengths = sorted(rows(&bare).iter().map(hops).collect::<Vec<_>>());
    assert_eq!(
        lengths,
        vec![2, 2, 5],
        "legacy `*` means {{1,}} capped at DEFAULT_MAX, so every s->t route: {lengths:?}"
    );

    // The whole justification for keeping the old spelling is that it is never
    // accepted SILENTLY. The diagnostic is part of the feature, not decoration,
    // so it is asserted against the server's own log rather than trusted.
    let log = std::fs::read_to_string(format!("/tmp/raisin-test-server-{HTTP_PORT}.log"))
        .expect("server log");
    assert!(
        log.contains("deprecated quantifier *1..2") && log.contains("write ->{1,2}"),
        "the legacy quantifier must produce a deprecation warning naming the \
         canonical spelling; nothing matching that appeared in the server log"
    );

    println!("[PASS] `*1..2` == `->{{1,2}}`, bare `*` is Q-SCOPE-exempt, deprecation warned");
}

// -------------------------------------------------------- 7. rule Q-SCOPE

/// An unbounded quantifier in the canonical form must sit under an explicit
/// selector or restrictor, and the error has to name both remedies — it is the
/// entire user experience of the rule.
async fn unbounded_quantifier_needs_a_selector_or_restrictor(base: &str, token: &str) {
    println!("--- rule Q-SCOPE ---");

    for form in ["*", "+", "{2,}"] {
        let err = sql(
            base,
            token,
            &format!(
                "SELECT * FROM GRAPH_TABLE(MATCH (a:Hub)-[e:link]->{form}(b:Hub) \
                   COLUMNS (a.id AS src))"
            ),
        )
        .await
        .expect_err(&format!(
            "`->{form}` with neither selector nor restrictor must be rejected"
        ));

        assert!(
            err.contains("unbounded quantifier"),
            "the error must say what is wrong, got: {err}"
        );
        // The rule is useless if the message only restates it: both remedies
        // must be named concretely.
        assert!(
            err.contains("ANY SHORTEST"),
            "the error must offer the selector remedy, got: {err}"
        );
        assert!(
            err.contains("TRAIL"),
            "the error must offer the restrictor remedy, got: {err}"
        );
    }

    println!("[PASS] all three unbounded forms rejected, each naming both remedies");
}

// -------------------------------------------------- 8. accessors on a cycle

/// `is_trail` / `is_acyclic` must disagree on a route that revisits a node, and
/// `path_first` / `path_last` must name the real endpoints.
async fn accessors_on_a_cycle(base: &str, token: &str) {
    println!("--- accessors on a cycle ---");

    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH TRAIL p = (a:Hub)-[e:ring]->{2,2}(b:Hub) \
           WHERE a.id = 'r1' \
           COLUMNS (path_length(p) AS hops, nodes(p) AS ns, is_trail(p) AS trail, \
                    is_acyclic(p) AS acyclic, path_first(p) AS head, path_last(p) AS tail))",
    )
    .await
    .expect("TRAIL on the ring failed");

    let matched = rows(&response);
    assert_eq!(
        matched.len(),
        1,
        "r1 -> r2 -> r1 is the only 2-hop trail: {matched:?}"
    );
    let row = &matched[0];
    assert_eq!(node_ids(row, "ns"), vec!["r1", "r2", "r1"], "{row}");
    assert_eq!(row["trail"], json!(true), "two distinct edges: {row}");
    assert_eq!(row["acyclic"], json!(false), "r1 is revisited: {row}");
    assert_eq!(row["head"]["id"], json!("r1"), "{row}");
    assert_eq!(row["tail"]["id"], json!("r1"), "{row}");
    println!("[PASS] is_trail=true, is_acyclic=false on r1->r2->r1");
}

// ------------------------------------------------------ 9. transport parity

/// The same path query over HTTP, WebSocket and PGWire (both protocol paths).
///
/// Three accessor result types are carried, one per `SqlValue` variant a path
/// lands on: `path_length` (Integer), `element_id` (String) and `nodes` (Json).
/// `element_id` is the whole ordered route in one scalar, so comparing it
/// compares the *route*, not merely the row count; `nodes` is included because a
/// JSON-valued column is the one a transport is most likely to mangle.
async fn transport_parity(base: &str, token: &str) {
    println!("--- HTTP / WebSocket / PGWire parity ---");

    let query = "SELECT * FROM GRAPH_TABLE(MATCH ANY CHEAPEST p = \
         (a:Hub)-[e:link COST e.weight]->{1,6}(b:Hub) \
         WHERE a.id = 's' AND b.id = 't' \
         COLUMNS (path_length(p) AS hops, element_id(p) AS eid, nodes(p) AS ns))";

    let expected_eid =
        "graph:s|link|graph:c1|link|graph:c2|link|graph:c3|link|graph:c4|link|graph:t";
    let expected_ids = vec!["s", "c1", "c2", "c3", "c4", "t"];

    let http = rows(&sql(base, token, query).await.expect("HTTP path query"));
    assert_eq!(http.len(), 1);
    assert_eq!(http[0]["eid"].as_str(), Some(expected_eid), "{}", http[0]);
    assert_eq!(node_ids(&http[0], "ns"), expected_ids, "{}", http[0]);

    let ws_result = sql_ws(HTTP_PORT, REPO, BRANCH, query)
        .await
        .unwrap_or_else(|e| panic!("WebSocket path query failed: {e}"));
    let ws = rows(&ws_result);
    assert_eq!(ws.len(), 1, "WebSocket row count differs: {ws:?}");
    assert_eq!(
        ws[0]["eid"].as_str(),
        Some(expected_eid),
        "WebSocket returned a different route: {}",
        ws[0]
    );
    assert_eq!(
        ws[0]["hops"].as_i64(),
        http[0]["hops"].as_i64(),
        "WebSocket and HTTP disagree on the hop count"
    );
    assert_eq!(
        ws[0]["ns"], http[0]["ns"],
        "WebSocket and HTTP disagree on the nodes(p) JSON"
    );

    // PGWire needs an API key for the connection and a real identity for reads
    // under RLS; see `grant_pgwire_identity`.
    let api_key = create_api_key(base, token).await;
    let pg = pg_wait_ready(PGWIRE_PORT, REPO, &api_key).await;
    let user_id = admin_user_id(base, token).await;
    grant_pgwire_identity(base, token, REPO, &user_id).await;
    pg.simple_query(&format!("SET app.user = '{token}'"))
        .await
        .unwrap_or_else(|e| panic!("SET app.user failed: {}", pg_error(&e)));

    let simple = pg_simple(&pg, query).await;
    assert_eq!(simple.len(), 1, "PGWire/simple row count: {simple:?}");
    let (pg_hops, pg_eid, pg_ns) = &simple[0];
    assert_eq!(
        pg_eid, expected_eid,
        "PGWire (simple query) returned a different route"
    );
    assert_eq!(
        pg_hops,
        &http[0]["hops"].as_i64().unwrap().to_string(),
        "PGWire (simple query) disagrees on the hop count"
    );
    // The simple-query protocol has no binary format — every value is text — so
    // the JSON accessor arrives as a string that must still parse into the same
    // node sequence. Comparing after parsing is the postgres protocol, not a
    // RaisinDB divergence.
    assert_eq!(
        ids_of(&parse_json("PGWire/simple", pg_ns)),
        expected_ids,
        "PGWire (simple query) returned different nodes(p)"
    );

    let extended = pg
        .query(query, &[])
        .await
        .unwrap_or_else(|e| panic!("PGWire extended query failed: {}", pg_error(&e)));
    assert_eq!(extended.len(), 1, "PGWire/extended row count");
    // By index, not by name: whether GRAPH_TABLE columns reach the wire bare or
    // table-qualified is incidental, and pinning it here would make this test
    // fail for the wrong reason.
    let eid: String = extended[0].get(1);
    assert_eq!(
        eid, expected_eid,
        "PGWire (extended/prepared) returned a different route"
    );
    println!("[PASS] HTTP, WebSocket and PGWire (simple + extended) agree on the route");

    pgwire_extended_json_column(&extended[0], &expected_ids);
}

/// PGWire extended protocol, JSON-valued column: **a known transport gap, and
/// not a path-support one**. Probed and printed rather than asserted, so it is
/// recorded where someone will see it instead of being quietly skipped.
///
/// The extended protocol describes a statement's columns *before* execution,
/// from the analyzer (`extended_query/schema.rs::describe_sql_columns`), and
/// encodes the `DataRow` *after* execution from the value
/// (`do_query` → `infer_schema_from_rows`). For `SELECT * FROM GRAPH_TABLE(...)`
/// the analyzer cannot type the table function's columns, so it describes them
/// `TEXT`, while `nodes(p)` produces an array that the value mapping types
/// `JSONB` and — because `tokio-postgres` requests the binary format — encodes
/// with PostgreSQL's `0x01` JSONB version byte. The client is holding `TEXT`, so
/// it hands back a string with a stray leading `\x01` that will not parse.
///
/// It is the same describe-vs-encode drift the geometry parity suite exists to
/// prevent (`spatial_transport_parity_test`), at a second site and with a
/// different cause: geometry agrees because both sides say `JSONB`, and nothing
/// can make the two agree here until the analyzer can type a table function's
/// columns. `element_id` is unaffected — `TEXT` on both sides — which is why the
/// route assertion above holds and is the thing the brief asks for.
///
/// Recorded in `docs/OPEN-ITEMS.md` §2.116.
fn pgwire_extended_json_column(row: &tokio_postgres::Row, expected_ids: &[&str]) {
    if let Ok(value) = row.try_get::<_, Value>(2) {
        assert_eq!(
            ids_of(&value),
            expected_ids,
            "PGWire (extended) decoded nodes(p) as JSON but with a different route"
        );
        println!("[NOTE] PGWire extended now decodes nodes(p) as JSON — OPEN-ITEMS §2.116 is fixed; turn this into an assertion.");
        return;
    }

    let text: String = match row.try_get::<_, String>(2) {
        Ok(text) => text,
        Err(e) => {
            println!("[KNOWN GAP] PGWire extended: nodes(p) is neither JSON nor text ({e}); OPEN-ITEMS §2.116");
            return;
        }
    };

    match serde_json::from_str::<Value>(&text) {
        Ok(value) => {
            assert_eq!(
                ids_of(&value),
                expected_ids,
                "PGWire (extended) returned nodes(p) as text with a different route"
            );
            println!("[NOTE] PGWire extended returns nodes(p) as parseable text — OPEN-ITEMS §2.116 is fixed; turn this into an assertion.");
        }
        Err(e) => println!(
            "[KNOWN GAP] PGWire extended: nodes(p) described TEXT but encoded JSONB-binary, \
             so it arrives unparseable ({e}); leading bytes {:?}. See OPEN-ITEMS §2.116. \
             element_id and the route itself are unaffected.",
            text.as_bytes().iter().take(4).collect::<Vec<_>>()
        ),
    }
}

/// `(hops, eid, nodes)` from the simple-query protocol, where every value is text.
async fn pg_simple(client: &tokio_postgres::Client, query: &str) -> Vec<(String, String, String)> {
    use tokio_postgres::SimpleQueryMessage;
    let messages = client
        .simple_query(query)
        .await
        .unwrap_or_else(|e| panic!("PGWire simple_query failed: {}", pg_error(&e)));
    let mut out = Vec::new();
    for message in messages {
        if let SimpleQueryMessage::Row(row) = message {
            out.push((
                row.get(0).unwrap_or_default().to_string(),
                row.get(1).unwrap_or_default().to_string(),
                row.get(2).unwrap_or_default().to_string(),
            ));
        }
    }
    out
}

fn parse_json(transport: &str, text: &str) -> Value {
    serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("{transport}: nodes(p) is not parseable JSON ({e}): {text}"))
}

fn ids_of(nodes: &Value) -> Vec<&str> {
    nodes
        .as_array()
        .unwrap_or_else(|| panic!("nodes(p) is not an array: {nodes}"))
        .iter()
        .map(|n| n["id"].as_str().unwrap_or("<no id>"))
        .collect()
}
