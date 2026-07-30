// Full-stack SQL/PGQ path test.
//
// Starts a real server, writes real nodes and real relations, and reads path
// results back through POST /api/sql/{repo}. Everything asserted here goes
// through the HTTP transport, so the JSON shapes are the ones a client sees.
//
// Covers:
//   * path variables and the accessor set (path_length / nodes / edges /
//     element_id / path_first / path_last / is_trail / is_acyclic)
//   * ANY SHORTEST (fewest hops) vs ANY CHEAPEST (lowest total weight)
//   * TRAIL and WALK restrictors on a cyclic graph
//   * `COLUMNS (p)` being a clear error rather than a lossy value
//   * ANY CHEAPEST over an unweighted edge erroring instead of silently
//     answering with a hop count
//   * the multi-type alternation regression: `-[:a|b]->` used to push only the
//     FIRST type down to storage and silently drop every `b` edge

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const REPO: &str = "pgq_path_test";
const BRANCH: &str = "main";
const WORKSPACE: &str = "graph";
const NODE_TYPE: &str = "pgq:City";

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
///
/// The GRAPH_TABLE table function emits columns as `graph_table.<name>`; which
/// of the two spellings reaches the client depends on the projection, so the
/// test accepts either rather than pinning an incidental detail.
fn rows(response: &Value) -> Vec<Value> {
    response["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("no rows in response: {response}"))
        .iter()
        .map(|row| match row.as_object() {
            Some(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let name = k.rsplit('.').next().unwrap_or(k).to_string();
                        (name, v.clone())
                    })
                    .collect(),
            ),
            None => row.clone(),
        })
        .collect()
}

async fn create_city(base_url: &str, token: &str, id: &str) {
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
    .unwrap_or_else(|e| panic!("failed to create city {id}: {e}"));
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
    let server = ServerHandle::start(ServerConfig::new(8104))
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

/// Build the fixture graph.
///
/// ```text
///                 road w=5          road w=5
///        a ------------------ b ---------------- d
///        |                                        ^
///        | road w=1    road w=1      road w=1     |
///        +--------- c1 --------- c2 -------------+
///
///        a -[knows]-> k        (single-type edge)
///        a -[follows]-> f      (the type the old code dropped)
///        x -[loop]-> y -[loop]-> x   (a 2-cycle, for TRAIL vs WALK)
/// ```
///
/// So a -> d has a 2-hop route costing 10 and a 3-hop route costing 3: fewest
/// hops and lowest cost are DIFFERENT answers, which is the only way to tell
/// ANY SHORTEST and ANY CHEAPEST apart.
async fn seed_graph(base_url: &str, token: &str) {
    for id in ["a", "b", "c1", "c2", "d", "k", "f", "x", "y"] {
        create_city(base_url, token, id).await;
    }

    add_relation(base_url, token, "a", "b", "road", Some(5.0)).await;
    add_relation(base_url, token, "b", "d", "road", Some(5.0)).await;
    add_relation(base_url, token, "a", "c1", "road", Some(1.0)).await;
    add_relation(base_url, token, "c1", "c2", "road", Some(1.0)).await;
    add_relation(base_url, token, "c2", "d", "road", Some(1.0)).await;

    add_relation(base_url, token, "a", "k", "knows", Some(1.0)).await;
    add_relation(base_url, token, "a", "f", "follows", Some(1.0)).await;

    add_relation(base_url, token, "x", "y", "loop", Some(1.0)).await;
    add_relation(base_url, token, "y", "x", "loop", Some(1.0)).await;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all pgq_path_e2e_test -- --ignored --nocapture
async fn pgq_paths_end_to_end() {
    println!("\n=== SQL/PGQ path end-to-end ===\n");

    let (server, token) = start_server().await;
    let base = server.base_url.clone();

    http_post(
        &base,
        "/api/repositories",
        &token,
        json!({ "repo_id": REPO, "description": "PGQ path test", "default_branch": BRANCH }),
    )
    .await
    .expect("failed to create repository");

    http_put(
        &base,
        &format!("/api/workspaces/{}/{}", REPO, WORKSPACE),
        &token,
        json!({
            "name": WORKSPACE,
            "description": "Path test graph",
            // raisin:Folder must be an allowed root type or workspace
            // provisioning fails.
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
                "description": "A city in the path test graph",
                "properties": [{ "name": "title", "type": "String", "required": false }],
                "allowed_children": []
            },
            "commit": { "message": "Create pgq:City", "actor": "test" }
        }),
    )
    .await
    .expect("failed to create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    seed_graph(&base, &token).await;
    println!("[OK] fixture graph seeded");

    multi_type_alternation_keeps_both_types(&base, &token).await;
    accessors_return_the_ordered_path(&base, &token).await;
    any_shortest_and_any_cheapest_disagree(&base, &token).await;
    selecting_a_path_directly_is_a_clear_error(&base, &token).await;
    cheapest_over_an_unweighted_edge_errors(&base, &token).await;
    trail_and_walk_differ_on_a_cycle(&base, &token).await;
    cardinality_still_reports_hop_count(&base, &token).await;
    a_selector_on_a_fixed_length_hop_is_rejected(&base, &token).await;

    println!("\n=== SQL/PGQ path end-to-end PASSED ===\n");
}

/// The regression: `-[:knows|follows]->` used to push only `types.first()` down
/// to `scan_relations_global`, so every `follows` edge vanished and the query
/// returned a strict subset of the truth with no error anywhere.
async fn multi_type_alternation_keeps_both_types(base: &str, token: &str) {
    println!("--- multi-type alternation ---");
    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH (a:PgqCity)-[r:knows|follows]->{1,1}(b:PgqCity) \
           COLUMNS (a.id AS src, b.id AS dst))",
    )
    .await
    .expect("alternation query failed");

    let matched = rows(&response);
    let reached: Vec<&str> = matched
        .iter()
        .filter(|row| row["src"] == json!("a"))
        .filter_map(|row| row["dst"].as_str())
        .collect();

    assert!(
        reached.contains(&"k"),
        "the FIRST alternation type is missing: {reached:?}"
    );
    assert!(
        reached.contains(&"f"),
        "the SECOND alternation type was dropped - this is the bug this test \
         exists for: {reached:?}"
    );
    println!("[PASS] both 'knows' and 'follows' survived: {reached:?}");
}

/// The ordered path used to be computed and discarded. Assert every accessor.
async fn accessors_return_the_ordered_path(base: &str, token: &str) {
    println!("--- path accessors ---");
    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:PgqCity)-[r:road]->{1,4}(b:PgqCity) \
           WHERE a.id = 'a' AND b.id = 'd' \
           COLUMNS (path_length(p) AS hops, nodes(p) AS ns, edges(p) AS es, \
                    element_id(p) AS eid, path_first(p) AS head, path_last(p) AS tail, \
                    is_trail(p) AS trail, is_acyclic(p) AS acyclic))",
    )
    .await
    .expect("accessor query failed");

    let matched = rows(&response);
    assert_eq!(
        matched.len(),
        1,
        "ANY SHORTEST must yield one row: {matched:?}"
    );
    let row = &matched[0];

    let hops = row["hops"].as_i64().expect("hops must be an integer");
    assert_eq!(hops, 2, "a->b->d is the fewest-hop route: {row}");

    let nodes = row["ns"].as_array().expect("nodes(p) must be an array");
    assert_eq!(
        nodes.len() as i64,
        hops + 1,
        "nodes(p) must be path_length + 1: {row}"
    );
    let node_ids: Vec<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();
    assert_eq!(node_ids, vec!["a", "b", "d"], "nodes must be in path order");
    assert!(
        nodes[0]["workspace"].is_string() && nodes[0]["node_type"].is_string(),
        "node identity shape is {{id, workspace, node_type}}: {}",
        nodes[0]
    );

    let edges = row["es"].as_array().expect("edges(p) must be an array");
    assert_eq!(edges.len() as i64, hops, "edges(p) must be path_length");
    assert_eq!(edges[0]["relation_type"], json!("road"));
    assert_eq!(edges[0]["source_id"], json!("a"));
    assert_eq!(edges[0]["target_id"], json!("b"));
    // The relation type must be verbatim - it used to be rewritten to "road[2]"
    // to smuggle the hop count out.
    assert!(
        !edges[0]["relation_type"].as_str().unwrap().contains('['),
        "relation_type must not carry a length encoding: {}",
        edges[0]
    );

    assert_eq!(row["head"]["id"], json!("a"));
    assert_eq!(row["tail"]["id"], json!("d"));
    assert_eq!(row["trail"], json!(true));
    assert_eq!(row["acyclic"], json!(true));

    let eid = row["eid"].as_str().expect("element_id must be text");
    assert!(
        eid.contains("road") && eid.contains(":a") && eid.contains(":d"),
        "element_id should encode the whole path, got {eid}"
    );
    println!("[PASS] hops={hops} nodes={node_ids:?} element_id={eid}");
}

/// Fewest hops and lowest cost are different routes in the fixture, which is
/// the only way to prove ANY CHEAPEST is not silently answering hop count.
async fn any_shortest_and_any_cheapest_disagree(base: &str, token: &str) {
    println!("--- ANY SHORTEST vs ANY CHEAPEST ---");

    let shortest = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:PgqCity)-[r:road]->{1,4}(b:PgqCity) \
           WHERE a.id = 'a' AND b.id = 'd' \
           COLUMNS (path_length(p) AS hops, element_id(p) AS eid))",
    )
    .await
    .expect("ANY SHORTEST failed");
    let shortest_rows = rows(&shortest);
    assert_eq!(shortest_rows.len(), 1);
    assert_eq!(
        shortest_rows[0]["hops"].as_i64(),
        Some(2),
        "ANY SHORTEST must take the 2-hop route: {}",
        shortest_rows[0]
    );

    let cheapest = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY CHEAPEST p = (a:PgqCity)-[t:road COST t.weight]->{1,4}(b:PgqCity) \
           WHERE a.id = 'a' AND b.id = 'd' \
           COLUMNS (path_length(p) AS hops, element_id(p) AS eid))",
    )
    .await
    .expect("ANY CHEAPEST failed");
    let cheapest_rows = rows(&cheapest);
    assert_eq!(cheapest_rows.len(), 1);
    assert_eq!(
        cheapest_rows[0]["hops"].as_i64(),
        Some(3),
        "ANY CHEAPEST must take the 3-hop, weight-3 route, not the 2-hop \
         weight-10 one - answering 2 here means the weight was ignored: {}",
        cheapest_rows[0]
    );

    println!("[PASS] shortest=2 hops, cheapest=3 hops (weights respected)");
}

/// There is no PATH column type. `COLUMNS (p)` must fail with a message naming
/// the accessors, not return something lossy.
async fn selecting_a_path_directly_is_a_clear_error(base: &str, token: &str) {
    println!("--- COLUMNS (p) ---");
    let err = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:PgqCity)-[r:road]->{1,4}(b:PgqCity) \
           WHERE a.id = 'a' AND b.id = 'd' \
           COLUMNS (p))",
    )
    .await
    .expect_err("selecting a path directly must be an error");

    assert!(
        err.contains("path_length") || err.contains("accessor"),
        "the error must name the accessors, got: {err}"
    );
    println!("[PASS] error names the accessors");
}

/// A missing weight under ANY CHEAPEST must be an error, never `unwrap_or(1.0)`.
/// The 'knows' edge here is weighted, so this uses a route that includes a
/// deliberately unweighted edge.
async fn cheapest_over_an_unweighted_edge_errors(base: &str, token: &str) {
    println!("--- ANY CHEAPEST over an unweighted edge ---");

    add_relation(base, token, "d", "k", "road", None).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let result = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY CHEAPEST p = (a:PgqCity)-[t:road COST t.weight]->{1,5}(b:PgqCity) \
           WHERE a.id = 'a' AND b.id = 'k' \
           COLUMNS (path_length(p) AS hops))",
    )
    .await;

    match result {
        Err(err) => {
            assert!(
                err.contains("weight"),
                "the error must be about the missing weight, got: {err}"
            );
            println!("[PASS] unweighted edge rejected: {err}");
        }
        Ok(response) => panic!(
            "ANY CHEAPEST silently answered over an unweighted edge - that is \
             the hop-count-masquerading-as-cost bug: {response}"
        ),
    }
}

/// ACYCLIC (the default) cannot traverse x->y->x. TRAIL can, because the two
/// edges are distinct. WALK can go round repeatedly.
async fn trail_and_walk_differ_on_a_cycle(base: &str, token: &str) {
    println!("--- TRAIL vs WALK vs the ACYCLIC default ---");

    let longest = |response: &Value| -> i64 {
        rows(response)
            .iter()
            .filter_map(|r| r["hops"].as_i64())
            .max()
            .unwrap_or(0)
    };

    let default = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH p = (a:PgqCity)-[r:loop]->{1,4}(b:PgqCity) \
           WHERE a.id = 'x' \
           COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect("default restrictor query failed");
    assert_eq!(
        longest(&default),
        1,
        "the ACYCLIC default must not revisit x: {default}"
    );

    let trail = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH TRAIL p = (a:PgqCity)-[r:loop]->{1,4}(b:PgqCity) \
           WHERE a.id = 'x' \
           COLUMNS (path_length(p) AS hops, is_acyclic(p) AS acyclic))",
    )
    .await
    .expect("TRAIL query failed");
    assert_eq!(
        longest(&trail),
        2,
        "TRAIL must allow x->y->x (two distinct edges): {trail}"
    );
    assert!(
        rows(&trail).iter().any(|r| r["acyclic"] == json!(false)),
        "the 2-hop TRAIL path revisits x, so is_acyclic must be false: {trail}"
    );

    let walk = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH WALK p = (a:PgqCity)-[r:loop]->{1,4}(b:PgqCity) \
           WHERE a.id = 'x' \
           COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect("WALK query failed");
    assert_eq!(
        longest(&walk),
        4,
        "WALK is bounded only by the quantifier: {walk}"
    );

    println!("[PASS] ACYCLIC=1 hop, TRAIL=2 hops, WALK=4 hops");
}

/// `CARDINALITY(r)` used to parse the hop count out of a mangled relation type.
/// It now reads the bound path; the answer must be unchanged.
async fn cardinality_still_reports_hop_count(base: &str, token: &str) {
    println!("--- CARDINALITY ---");
    let response = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST p = (a:PgqCity)-[r:road]->{1,4}(b:PgqCity) \
           WHERE a.id = 'a' AND b.id = 'd' \
           COLUMNS (CARDINALITY(r) AS hops, r.type AS rel_type))",
    )
    .await
    .expect("CARDINALITY query failed");

    let matched = rows(&response);
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0]["hops"].as_i64(), Some(2), "{}", matched[0]);
    assert_eq!(
        matched[0]["rel_type"],
        json!("road"),
        "the relation type must be verbatim, not 'road[2]': {}",
        matched[0]
    );
    println!("[PASS] CARDINALITY=2 with relation type 'road'");
}

/// Only a variable-length pattern produces paths. A selector or path variable on
/// a fixed-length hop must be REJECTED rather than quietly matched as an
/// ordinary hop — answering a different question than the one asked, with no
/// error anywhere, is the failure mode this whole pass exists to remove.
async fn a_selector_on_a_fixed_length_hop_is_rejected(base: &str, token: &str) {
    println!("--- selector on a fixed-length hop ---");

    let err = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH ANY SHORTEST (a:PgqCity)-[r:road]->(b:PgqCity) \
           COLUMNS (a.id AS src))",
    )
    .await
    .expect_err("a selector on a fixed-length hop must be rejected");
    assert!(
        err.contains("variable-length"),
        "the error must say a quantifier is required, got: {err}"
    );

    let err = sql(
        base,
        token,
        "SELECT * FROM GRAPH_TABLE(MATCH p = (a:PgqCity)-[r:road]->(b:PgqCity)-[s:road]->(c:PgqCity) \
           COLUMNS (path_length(p) AS hops))",
    )
    .await
    .expect_err("a path variable on a multi-hop chain must be rejected");
    assert!(
        err.contains("variable-length"),
        "the error must say a quantifier is required, got: {err}"
    );

    println!("[PASS] both rejected with a message naming the remedy");
}
