// Full-stack test for the SQL/PGQ adjacency relation-type scope.
//
// Starts a real server, writes real nodes and real relations, and reads results
// back through POST /api/sql/{repo}.
//
// `build_adjacency_with_weights` used to pass `None` to `scan_relations_global`,
// so every scalar graph function (pagerank, wcc, component_count, degree, …)
// loaded EVERY relation in the branch across ALL workspaces on EVERY
// invocation, and answered over the whole branch regardless of what the MATCH
// clause said. The scope now comes from the query's own patterns.
//
// That is a numeric behaviour change, so it needs proof at the SQL surface
// rather than a unit test asserting an internal signature. The fixture is built
// so the two readings give DIFFERENT answers:
//
//   road:    a -> b -> c        (one weakly connected component of 3)
//   ferry:   d -> e             (a second, disjoint component of 2)
//
// Over the `road` graph alone `a` and `c` share a component and `d` does not
// exist. Over the whole branch there are two components. A function that still
// ignores the pattern's types cannot tell those apart.
//
// Also covers, because they are the two ways the scope could be WRONG rather
// than merely wide:
//   * an untyped hop anywhere in the pattern must widen the scope back to the
//     whole branch — narrowing on a partially-typed pattern would drop edges
//     the untyped hop is entitled to match;
//   * a multi-type alternation must keep BOTH types, since a single type is
//     pushed into storage and several are filtered in memory.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const REPO: &str = "pgq_scope_test";
const BRANCH: &str = "main";
const WORKSPACE: &str = "graph";
const NODE_TYPE: &str = "scope:Stop";

async fn http_post(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let client = Client::new();
    let response = client
        .post(format!("{}{}", base_url, path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{}: {}", status, text));
    }
    serde_json::from_str(&text).map_err(|_| text)
}

async fn http_put(base_url: &str, path: &str, token: &str, body: Value) -> Result<(), String> {
    let client = Client::new();
    let response = client
        .put(format!("{}{}", base_url, path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("{}: {}", status, body));
    }
    Ok(())
}

async fn sql(base_url: &str, token: &str, query: &str) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{}", REPO),
        token,
        json!({ "sql": query, "params": [] }),
    )
    .await
}

/// Rows of a successful SQL response, with any table qualifier stripped.
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

async fn create_stop(base_url: &str, token: &str, id: &str) {
    http_post(
        base_url,
        &format!("/api/repository/{}/{}/head/{}/", REPO, BRANCH, WORKSPACE),
        token,
        json!({
            "node": {
                "id": id,
                "name": id,
                "node_type": NODE_TYPE,
                "properties": { "title": id }
            }
        }),
    )
    .await
    .unwrap_or_else(|e| panic!("failed to create stop {id}: {e}"));
}

async fn add_relation(
    base_url: &str,
    token: &str,
    source: &str,
    target: &str,
    relation_type: &str,
    weight: Option<f32>,
) {
    let mut body = json!({
        "targetWorkspace": WORKSPACE,
        "targetPath": format!("/{}", target),
        "relationType": relation_type,
    });
    if let Some(w) = weight {
        body["weight"] = json!(w);
    }

    http_post(
        base_url,
        &format!(
            "/api/repository/{}/{}/head/{}/{}/raisin:cmd/add-relation",
            REPO, BRANCH, WORKSPACE, source
        ),
        token,
        body,
    )
    .await
    .unwrap_or_else(|e| panic!("failed to relate {source} -{relation_type}-> {target}: {e}"));
}

/// Bring up a server with an authenticated admin token.
async fn start_server() -> (ServerHandle, String) {
    let server = ServerHandle::start(ServerConfig::new(8105))
        .await
        .expect("failed to start server");

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("failed to authenticate");

    // Clear must_change_password so the token is usable for data operations.
    let client = Client::new();
    let profile = client
        .get(format!("{}/api/raisindb/me", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap();
    client
        .put(format!(
            "{}/api/raisindb/sys/default/users/{}",
            server.base_url, user_id
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await
        .unwrap();

    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("failed to re-authenticate");

    (server, token)
}

/// Two disjoint subgraphs distinguished only by relation type.
///
/// ```text
///   road:   a -> b -> c
///   ferry:  d -> e
///   walk:   a -> z          (a third type, so alternation has something to drop)
/// ```
async fn seed_graph(base_url: &str, token: &str) {
    for id in ["a", "b", "c", "d", "e", "z"] {
        create_stop(base_url, token, id).await;
    }

    add_relation(base_url, token, "a", "b", "road", Some(1.0)).await;
    add_relation(base_url, token, "b", "c", "road", Some(1.0)).await;
    add_relation(base_url, token, "d", "e", "ferry", Some(1.0)).await;
    add_relation(base_url, token, "a", "z", "walk", Some(1.0)).await;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all pgq_adjacency_scope_e2e_test -- --ignored --nocapture
async fn pgq_adjacency_scope_end_to_end() {
    println!("\n=== SQL/PGQ adjacency relation-type scope end-to-end ===\n");

    let (server, token) = start_server().await;
    let base = server.base_url.clone();

    http_post(
        &base,
        "/api/repositories",
        &token,
        json!({ "repo_id": REPO, "description": "PGQ scope test", "default_branch": BRANCH }),
    )
    .await
    .expect("failed to create repository");

    http_put(
        &base,
        &format!("/api/workspaces/{}/{}", REPO, WORKSPACE),
        &token,
        json!({
            "name": WORKSPACE,
            "description": "Scope test graph",
            "allowed_node_types": [NODE_TYPE, "raisin:Folder"],
            "allowed_root_node_types": [NODE_TYPE, "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("failed to create workspace");

    http_post(
        &base,
        &format!("/api/management/{}/{}/nodetypes", REPO, BRANCH),
        &token,
        json!({
            "node_type": {
                "name": NODE_TYPE,
                "description": "A stop in the scope test graph",
                "properties": [{ "name": "title", "type": "String", "required": false }],
                "allowed_children": []
            },
            "commit": { "message": "Create scope:Stop", "actor": "test" }
        }),
    )
    .await
    .expect("failed to create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    seed_graph(&base, &token).await;
    println!("[OK] fixture graph seeded");

    // Printed, not asserted: if an assertion below fails, these show whether it
    // was the scope or the surrounding query shape that misbehaved.
    for probe in [
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road]->(b) COLUMNS (a.id AS src, b.id AS dst))",
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r]->(b) COLUMNS (a.id AS src, b.id AS dst))",
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road]->(b) COLUMNS (a.id AS src, out_degree(a) AS d))",
    ] {
        println!("PROBE {probe}\n  => {:?}\n", sql(&base, &token, probe).await);
    }

    a_typed_pattern_scopes_the_algorithm_graph(&base, &token).await;
    an_untyped_hop_widens_the_scope_back_to_the_branch(&base, &token).await;
    an_alternation_keeps_every_named_type(&base, &token).await;
    alternation_binding_keeps_every_type(&base, &token).await;
    repeated_algorithms_in_one_columns_clause_agree(&base, &token).await;

    println!("\n=== SQL/PGQ adjacency scope end-to-end PASSED ===\n");
}

/// `out_degree(a)` for the given SQL, as a map from node id to degree.
async fn degrees_by_node(base: &str, token: &str, query: &str, label: &str) -> Vec<(String, i64)> {
    let response = sql(base, token, query)
        .await
        .unwrap_or_else(|e| panic!("{label} query failed: {e}"));
    let mut out: Vec<(String, i64)> = rows(&response)
        .iter()
        .map(|row| {
            (
                row["src"].as_str().unwrap_or_default().to_string(),
                row["outd"]
                    .as_i64()
                    .unwrap_or_else(|| panic!("{label}: no integer out_degree in {row}")),
            )
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The decisive case: `component_count()` under a `road`-only pattern must see
/// only the road graph.
///
/// Before the pushdown every graph function loaded the whole branch, so this
/// answered 2 (road + ferry) no matter what the pattern said.
async fn a_typed_pattern_scopes_the_algorithm_graph(base: &str, token: &str) {
    println!("--- a typed pattern scopes the algorithm graph ---");

    let scoped = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road]->(b) \
         COLUMNS (a.id AS src, component_count() AS components))",
    )
    .await
    .expect("road-scoped component_count failed");

    let scoped_rows = rows(&scoped);
    assert!(
        !scoped_rows.is_empty(),
        "the road pattern must match something: {scoped}"
    );
    let components = scoped_rows[0]["components"].as_i64();
    assert_eq!(
        components,
        Some(1),
        "under a road-only pattern the graph is the road graph, which has ONE \
         component (a-b-c). Seeing 2 means the ferry edge leaked in and the \
         relation-type scope is not being pushed down: {}",
        scoped_rows[0]
    );

    println!("[PASS] road-scoped component_count = 1");
}

/// The safety rule: an untyped hop must widen the scope back to everything.
///
/// Narrowing on a partially-typed pattern would drop edges the untyped hop is
/// entitled to match — a silent-wrong-results bug in the name of a faster scan.
async fn an_untyped_hop_widens_the_scope_back_to_the_branch(base: &str, token: &str) {
    println!("--- an untyped hop widens the scope ---");

    let widened = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r]->(b) \
         COLUMNS (a.id AS src, component_count() AS components))",
    )
    .await
    .expect("untyped component_count failed");

    let widened_rows = rows(&widened);
    assert!(!widened_rows.is_empty(), "untyped pattern matched nothing");
    assert_eq!(
        widened_rows[0]["components"].as_i64(),
        Some(2),
        "an untyped hop must see the WHOLE branch: road (a-b-c-z) and ferry \
         (d-e) are two components. Seeing 1 means the scope narrowed on a \
         pattern that never named its types: {}",
        widened_rows[0]
    );

    println!("[PASS] untyped pattern sees both components");
}

/// A multi-type alternation must widen the adjacency to every named type.
///
/// Only ONE type can be pushed into `scan_relations_global`, so an alternation
/// scans unfiltered and is filtered while the adjacency is built. Keeping just
/// the first type is how `-[:a|b]->` silently drops every `b` edge.
///
/// `out_degree(a)` reads the scoped adjacency directly, so it isolates the
/// adjacency builder from the *binding* side of alternation — which is broken
/// independently, see [`alternation_binding_keeps_every_type`].
async fn an_alternation_keeps_every_named_type(base: &str, token: &str) {
    println!("--- an alternation keeps every named type ---");

    // road ∪ walk out of `a`: a->b and a->z. Ferry must stay out.
    let both = degrees_by_node(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road|walk]->(b) \
         COLUMNS (a.id AS src, out_degree(a) AS outd))",
        "road|walk",
    )
    .await;

    let a_degree = both
        .iter()
        .find(|(id, _)| id == "a")
        .map(|(_, d)| *d)
        .unwrap_or_else(|| panic!("no row for 'a': {both:?}"));
    assert_eq!(
        a_degree, 2,
        "out_degree(a) over road|walk must count BOTH the road edge a->b and \
         the walk edge a->z. 1 means only the first alternation type reached \
         the adjacency, which is the silent-drop bug: {both:?}"
    );

    // The ferry component must not leak in: component_count over road|walk is
    // 1 ({a,b,c,z}); adding ferry would make it 2.
    let scoped = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road|walk]->(b) \
         COLUMNS (a.id AS src, component_count() AS components))",
    )
    .await
    .expect("road|walk component_count failed");
    let scoped_rows = rows(&scoped);
    assert_eq!(
        scoped_rows[0]["components"].as_i64(),
        Some(1),
        "the ferry edge leaked into a road|walk scope: {}",
        scoped_rows[0]
    );

    println!("[PASS] alternation adjacency kept road and walk, excluded ferry");
}

/// Single-hop *binding* over an alternation must keep every named type.
///
/// This was a known gap when the adjacency scope landed: `single_hop.rs` pushed
/// `rel_pattern.types.first()` into `scan_relations_global`, so the second
/// type's rows never came back from storage and there was no post-filter below
/// it to keep them — `-[:road|walk]->` silently bound only the road edges. The
/// variable-length matcher had the identical defect and was fixed first; this
/// site now matches it (single type pushed down, several filtered in memory),
/// so the probe is an assertion rather than a printed note.
async fn alternation_binding_keeps_every_type(base: &str, token: &str) {
    println!("--- single-hop alternation binding ---");

    let both = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road|walk]->(b) \
         COLUMNS (a.id AS src, b.id AS dst))",
    )
    .await
    .expect("alternation binding query failed");

    let matched = rows(&both);
    let pair = |src: &str, dst: &str| {
        matched
            .iter()
            .any(|row| row["src"].as_str() == Some(src) && row["dst"].as_str() == Some(dst))
    };

    assert!(
        pair("a", "b"),
        "the FIRST alternation type (road) is missing: {matched:?}"
    );
    assert!(
        pair("a", "z"),
        "the SECOND alternation type (walk) was dropped — that is the \
         types.first() pushdown bug: {matched:?}"
    );
    assert!(
        !pair("d", "e"),
        "the ferry edge is not in the alternation and must not bind: {matched:?}"
    );

    println!("[PASS] single-hop alternation bound road AND walk, and only those");
}

/// Several algorithms in one COLUMNS clause must agree with each other.
///
/// They now share ONE memoised adjacency per relation-type scope instead of
/// each rebuilding it; if the memo were keyed wrongly they would disagree, or
/// one would answer over a different graph than its neighbour.
async fn repeated_algorithms_in_one_columns_clause_agree(base: &str, token: &str) {
    println!("--- repeated algorithms in one COLUMNS clause ---");

    let combined = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a)-[r:road]->(b) \
         COLUMNS (a.id AS src, component_count() AS c1, out_degree(a) AS outd, \
         component_count() AS c2))",
    )
    .await
    .expect("combined algorithm query failed");

    let combined_rows = rows(&combined);
    assert!(!combined_rows.is_empty(), "road pattern matched nothing");

    for row in &combined_rows {
        assert_eq!(
            row["c1"], row["c2"],
            "two calls to component_count() in one COLUMNS clause disagreed - \
             the shared adjacency memo is keyed wrongly: {row}"
        );
        assert_eq!(
            row["c1"].as_i64(),
            Some(1),
            "the road graph still has one component: {row}"
        );
    }

    // `a` has one outgoing road edge (a->b); its `walk` edge is out of scope.
    let a_row = combined_rows
        .iter()
        .find(|row| row["src"].as_str() == Some("a"))
        .unwrap_or_else(|| panic!("no row for 'a': {combined_rows:?}"));
    assert_eq!(
        a_row["outd"].as_i64(),
        Some(1),
        "out_degree(a) must count road edges only - 2 means the walk edge was \
         still in the adjacency: {a_row}"
    );

    println!("[PASS] memoised adjacency is consistent across calls");
}
