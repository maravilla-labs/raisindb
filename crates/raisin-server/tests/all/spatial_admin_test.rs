//! End-to-end proof of the spatial index ADMIN + REINDEX surface, against a real
//! server over HTTP.
//!
//! # What this file is actually proving
//!
//! The spatial index has two sides that must be kept apart, and every bug this
//! subsystem has had came from confusing them:
//!
//! - **Configured intent** — `WorkspaceConfig.spatial`, replicated, written by
//!   `ALTER SPATIAL INDEX` or `PUT …/spatial/config`.
//! - **Local reality** — the per-node `INDEX_STATUS` record and the geohash keys
//!   physically on this disk.
//!
//! Changing intent must NOT change query results on its own. Only a rebuild moves
//! reality towards intent, and while it does, results must stay identical. Those
//! two sentences are the whole test.
//!
//! Run with:
//! `cargo test -p raisin-server --test all spatial_admin_test -- --ignored --nocapture`

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::{Client, StatusCode};
use serde_json::{json, Value};

const TENANT: &str = "default";
const BRANCH: &str = "main";
const WORKSPACE: &str = "places";

/// The full default precision set, `INDEX_PRECISIONS_DEFAULT`.
const DEFAULT_PRECISIONS: [u64; 8] = [11, 10, 9, 8, 7, 6, 4, 2];
/// A deliberately coarse set: still correctness-preserving (the cover algorithm
/// picks a cell at least as large as the query radius), just less selective.
const COARSE_PRECISIONS: [u64; 3] = [6, 4, 2];

/// Three points, far enough apart that a 5 km radius isolates exactly one.
const PLACES: [(&str, &str, f64, f64); 3] = [
    ("zurich", "Zurich HB", 8.5402, 47.3779),
    ("bern", "Bern HB", 7.4396, 46.9490),
    ("geneva", "Geneva Cornavin", 6.1420, 46.2100),
];

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn http_post(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let response = Client::new()
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

async fn http_put(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let response = Client::new()
        .put(format!("{}{}", base_url, path))
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
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn http_get(base_url: &str, path: &str, token: &str) -> Result<Value, String> {
    let response = Client::new()
        .get(format!("{}{}", base_url, path))
        .bearer_auth(token)
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

/// Raw status of a request, for the authorization assertions where the status IS
/// the thing under test.
async fn status_of(
    method: reqwest::Method,
    base_url: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> StatusCode {
    let mut req = Client::new().request(method, format!("{}{}", base_url, path));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(b) = body {
        req = req.json(&b);
    }
    req.send().await.expect("request failed").status()
}

async fn sql(base_url: &str, repo: &str, token: &str, statement: &str) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{}", repo),
        token,
        json!({ "sql": statement, "params": [] }),
    )
    .await
}

fn rows(response: &Value) -> &Vec<Value> {
    response["rows"].as_array().expect("rows array")
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Start a server, authenticate as the tenant admin, and create repo + workspace +
/// nodetype. Returns `(server, admin token)`.
async fn boot(port: u16, repo: &str) -> (ServerHandle, String) {
    let server = ServerHandle::start(ServerConfig::new(port))
        .await
        .expect("server start failed");

    // Admin user creation is asynchronous at boot.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let token = authenticate(&server.base_url, TENANT, "admin", "Admin12345!@#")
        .await
        .expect("authenticate failed");

    // Clear must_change_password, then re-authenticate for a clean token.
    let client = Client::new();
    let profile: Value = client
        .get(format!("{}/api/raisindb/me", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap().to_string();
    let _ = client
        .put(format!(
            "{}/api/raisindb/sys/{}/users/{}",
            server.base_url, TENANT, user_id
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await;

    let token = authenticate(&server.base_url, TENANT, "admin", "Admin12345!@#")
        .await
        .expect("re-authenticate failed");

    http_post(
        &server.base_url,
        "/api/repositories",
        &token,
        json!({ "repo_id": repo, "description": "spatial admin test", "default_branch": BRANCH }),
    )
    .await
    .expect("create repository");

    http_put(
        &server.base_url,
        &format!("/api/workspaces/{}/{}", repo, WORKSPACE),
        &token,
        json!({
            "name": WORKSPACE,
            "description": "places",
            // `raisin:Folder` must be allowed: creating a workspace materialises a
            // root folder node, so a workspace permitting only its own type is
            // rejected at creation time.
            "allowed_node_types": ["geo:Place", "raisin:Folder"],
            "allowed_root_node_types": ["geo:Place", "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create workspace");

    http_post(
        &server.base_url,
        &format!("/api/management/{}/{}/nodetypes", repo, BRANCH),
        &token,
        json!({
            "node_type": {
                "name": "geo:Place",
                "description": "A place with a location",
                "properties": [
                    { "name": "title", "type": "String", "required": true },
                    { "name": "location", "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "create geo:Place", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    (server, token)
}

/// Insert the three places **over SQL**, which is the interface that matters: a DML
/// path that bypassed the indexing hook would write data invisible to the index.
async fn insert_places(base_url: &str, repo: &str, token: &str) {
    for (id, title, lon, lat) in PLACES {
        let statement = format!(
            "INSERT INTO '{ws}' (id, name, node_type, path, properties) VALUES \
             ('{id}', '{id}', 'geo:Place', '/{id}', \
              '{{\"title\":\"{title}\",\"location\":{{\"type\":\"Point\",\"coordinates\":[{lon},{lat}]}}}}'::jsonb)",
            ws = WORKSPACE,
            id = id,
            title = title,
            lon = lon,
            lat = lat,
        );
        sql(base_url, repo, token, &statement)
            .await
            .unwrap_or_else(|e| panic!("INSERT {} failed: {}", id, e));
    }
}

/// Wait until the index census reports `nodes` distinct nodes at `precisions`
/// precisions each.
///
/// Polled rather than slept: the automatic reconcile job may be re-writing entries
/// concurrently with the inserts, so the census can legitimately observe a partial
/// state for a moment. A fixed sleep here made the assertion flaky, which is a worse
/// outcome than a slower test.
async fn await_indexed(
    base_url: &str,
    repo: &str,
    token: &str,
    nodes: usize,
    precisions: usize,
    what: &str,
) -> Value {
    let want_live = (nodes * precisions) as i64;
    for attempt in 0..60 {
        let row = health(base_url, repo, token).await;
        if row["live_entries"].as_i64() == Some(want_live)
            && row["distinct_nodes"].as_i64() == Some(nodes as i64)
        {
            println!(
                "[OK] {} after {} poll(s): live_entries={} distinct_nodes={}",
                what,
                attempt + 1,
                row["live_entries"],
                row["distinct_nodes"]
            );
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!(
        "{}: timed out waiting for {} live entries over {} nodes; last health = {}",
        what,
        want_live,
        nodes,
        health(base_url, repo, token).await
    );
}

// ---------------------------------------------------------------------------
// Observations
// ---------------------------------------------------------------------------

/// The single HEALTH row for `places`.`location`.
async fn health(base_url: &str, repo: &str, token: &str) -> Value {
    let r = sql(
        base_url,
        repo,
        token,
        &format!(
            "SHOW SPATIAL INDEX HEALTH FOR '{}' PROPERTY 'location'",
            WORKSPACE
        ),
    )
    .await
    .expect("SHOW SPATIAL INDEX HEALTH failed");
    let rows = rows(&r);
    assert_eq!(
        rows.len(),
        1,
        "expected exactly one health row for places.location, got {:?}",
        rows
    );
    rows[0].clone()
}

/// Parse `"11:3, 10:3, …"` into the precisions present, ascending.
fn precisions_present(health_row: &Value) -> Vec<u64> {
    let text = health_row["live_per_precision"].as_str().unwrap_or("");
    let mut out: Vec<u64> = text
        .split(',')
        .filter_map(|part| part.trim().split(':').next().and_then(|p| p.parse().ok()))
        .collect();
    out.sort_unstable();
    out
}

fn indexed_precisions(health_row: &Value) -> Vec<u64> {
    let text = health_row["indexed_precisions"].as_str().unwrap_or("()");
    let mut out: Vec<u64> = text
        .trim_matches(|c| c == '(' || c == ')')
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    out.sort_unstable();
    out
}

fn sorted(mut v: Vec<u64>) -> Vec<u64> {
    v.sort_unstable();
    v
}

/// Wait for the reindex job to land the expected precision set.
///
/// Polls rather than sleeping a fixed amount: the job is asynchronous and a fixed
/// sleep would make this test flaky on a loaded machine, which is worse than slow.
async fn await_precisions(
    base_url: &str,
    repo: &str,
    token: &str,
    expected: &[u64],
    what: &str,
) -> Value {
    let want = sorted(expected.to_vec());
    for attempt in 0..60 {
        let row = health(base_url, repo, token).await;
        if indexed_precisions(&row) == want && precisions_present(&row) == want {
            println!(
                "[OK] {} after {} poll(s): indexed={:?} on-disk={:?} live_entries={}",
                what,
                attempt + 1,
                indexed_precisions(&row),
                precisions_present(&row),
                row["live_entries"]
            );
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let row = health(base_url, repo, token).await;
    panic!(
        "{}: timed out waiting for precisions {:?}; last health row = {}",
        what, want, row
    );
}

/// The three names a 400 km radius around Bern must return, in distance order.
async fn radius_query(base_url: &str, repo: &str, token: &str, radius_m: u64) -> Vec<String> {
    let statement = format!(
        "SELECT name FROM '{ws}' \
         WHERE ST_DWITHIN(properties->>'location', ST_POINT(7.4396, 46.9490), {r}) \
         ORDER BY name",
        ws = WORKSPACE,
        r = radius_m
    );
    let r = sql(base_url, repo, token, &statement)
        .await
        .unwrap_or_else(|e| panic!("ST_DWITHIN query failed: {}", e));
    rows(&r)
        .iter()
        .map(|row| row["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// The main test
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // starts a real server; run with --ignored
async fn spatial_admin_config_and_reindex_end_to_end() {
    const REPO: &str = "spatial_admin_test";
    let (server, token) = boot(8231, REPO).await;
    let base = server.base_url.clone();
    println!("\n=== spatial admin + reindex e2e ===\n");

    // -- 1. Data written over SQL is indexed automatically, with no opt-in. ------
    insert_places(&base, REPO, &token).await;

    let row = await_indexed(
        &base,
        REPO,
        &token,
        PLACES.len(),
        DEFAULT_PRECISIONS.len(),
        "automatic indexing of SQL-written geometry",
    )
    .await;
    println!("[health after insert] {}", row);
    assert_eq!(
        indexed_precisions(&row),
        sorted(DEFAULT_PRECISIONS.to_vec()),
        "a fresh workspace must index at the default precision set with no admin action"
    );
    assert_eq!(row["needs_rebuild"], json!(false));

    let baseline_near = radius_query(&base, REPO, &token, 5_000).await;
    let baseline_wide = radius_query(&base, REPO, &token, 400_000).await;
    assert_eq!(
        baseline_near,
        vec!["bern".to_string()],
        "a 5 km radius around Bern must isolate Bern"
    );
    assert_eq!(
        baseline_wide,
        vec![
            "bern".to_string(),
            "geneva".to_string(),
            "zurich".to_string()
        ],
        "a 400 km radius must reach all three"
    );
    println!(
        "[OK] baseline queries: near={:?} wide={:?}",
        baseline_near, baseline_wide
    );

    // -- 2. VERIFY agrees that intent and reality match. ------------------------
    let r = sql(
        &base,
        REPO,
        &token,
        &format!(
            "VERIFY SPATIAL INDEX FOR '{}' PROPERTY 'location'",
            WORKSPACE
        ),
    )
    .await
    .expect("VERIFY failed");
    assert_eq!(
        rows(&r)[0]["status"].as_str().unwrap(),
        "OK",
        "{:?}",
        rows(&r)
    );
    println!("[OK] VERIFY reports OK on a freshly written index");

    // -- 3. Change the CONFIG over SQL. Intent moves; reality does not. ---------
    let r = sql(
        &base,
        REPO,
        &token,
        &format!(
            "ALTER SPATIAL INDEX FOR '{}' PROPERTY 'location' SET PRECISIONS = (6, 4, 2)",
            WORKSPACE
        ),
    )
    .await
    .expect("ALTER SPATIAL INDEX failed");
    println!("[alter] {}", rows(&r)[0]);

    let r = sql(
        &base,
        REPO,
        &token,
        &format!(
            "SHOW SPATIAL INDEX CONFIG FOR '{}' PROPERTY 'location'",
            WORKSPACE
        ),
    )
    .await
    .expect("SHOW SPATIAL INDEX CONFIG failed");
    assert_eq!(
        rows(&r)[0]["precisions"].as_str().unwrap(),
        "(6, 4, 2)",
        "the configured policy must reflect the ALTER"
    );

    let row = health(&base, REPO, &token).await;
    assert_eq!(
        indexed_precisions(&row),
        sorted(DEFAULT_PRECISIONS.to_vec()),
        "a config change alone must NOT rewrite the index"
    );
    assert_eq!(
        row["needs_rebuild"],
        json!(true),
        "the drift between intent and reality must be visible"
    );
    // The load-bearing assertion of the whole design: reconfiguration cannot make
    // data invisible, because queries plan against what is INDEXED, not configured.
    assert_eq!(
        radius_query(&base, REPO, &token, 5_000).await,
        baseline_near
    );
    assert_eq!(
        radius_query(&base, REPO, &token, 400_000).await,
        baseline_wide
    );
    println!("[OK] config changed, index untouched, results identical");

    // -- 4. REBUILD migrates the data that already existed. --------------------
    let r = sql(
        &base,
        REPO,
        &token,
        &format!(
            "REBUILD SPATIAL INDEX FOR '{}' PROPERTY 'location'",
            WORKSPACE
        ),
    )
    .await
    .expect("REBUILD SPATIAL INDEX failed");
    println!("[rebuild] {}", rows(&r)[0]);
    assert!(rows(&r)[0]["job_id"]
        .as_str()
        .is_some_and(|s| !s.is_empty()));

    let row = await_precisions(&base, REPO, &token, &COARSE_PRECISIONS, "shrink to (6,4,2)").await;
    assert_eq!(
        row["live_entries"].as_i64().unwrap(),
        (PLACES.len() * COARSE_PRECISIONS.len()) as i64,
        "the rebuild must remove the entries the new policy no longer wants"
    );
    assert_eq!(row["distinct_nodes"].as_i64().unwrap(), PLACES.len() as i64);
    assert_eq!(row["needs_rebuild"], json!(false));
    assert!(
        !precisions_present(&row).contains(&11),
        "precision 11 entries must be gone after shrinking the policy"
    );

    // Same rows, at every radius, across the migration. Precision is a performance
    // knob and must never be a correctness knob.
    assert_eq!(
        radius_query(&base, REPO, &token, 5_000).await,
        baseline_near
    );
    assert_eq!(
        radius_query(&base, REPO, &token, 400_000).await,
        baseline_wide
    );
    println!("[OK] reindex migrated pre-existing data; results unchanged");

    // -- 5. Idempotency: rebuilding twice must not duplicate or corrupt. --------
    let before = health(&base, REPO, &token).await;
    for round in 1..=2 {
        sql(
            &base,
            REPO,
            &token,
            &format!(
                "REBUILD SPATIAL INDEX FOR '{}' PROPERTY 'location'",
                WORKSPACE
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("idempotency rebuild {} failed: {}", round, e));
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    let after = await_precisions(
        &base,
        REPO,
        &token,
        &COARSE_PRECISIONS,
        "repeated rebuilds settle",
    )
    .await;
    assert_eq!(
        after["live_entries"], before["live_entries"],
        "repeated rebuilds must not add live entries"
    );
    assert_eq!(after["distinct_nodes"], before["distinct_nodes"]);
    assert_eq!(
        precisions_present(&after),
        precisions_present(&before),
        "repeated rebuilds must not change the per-precision distribution"
    );
    assert_eq!(
        radius_query(&base, REPO, &token, 400_000).await,
        baseline_wide
    );
    println!(
        "[OK] rebuild is idempotent: {} live entries before and after",
        after["live_entries"]
    );

    // -- 6. The same round trip over the HTTP admin endpoints. -----------------
    let cfg_path = format!(
        "/api/admin/management/database/{}/{}/spatial/config",
        TENANT, REPO
    );
    let put = http_put(
        &base,
        &cfg_path,
        &token,
        json!({
            "workspace": WORKSPACE,
            "property": "location",
            "precisions": DEFAULT_PRECISIONS,
        }),
    )
    .await
    .expect("PUT spatial/config failed");
    println!("[http put config] {}", put);
    assert_eq!(
        put["precisions"].as_array().unwrap().len(),
        DEFAULT_PRECISIONS.len()
    );

    // The HTTP write must be visible to SQL: one record, two surfaces.
    let r = sql(
        &base,
        REPO,
        &token,
        &format!(
            "SHOW SPATIAL INDEX CONFIG FOR '{}' PROPERTY 'location'",
            WORKSPACE
        ),
    )
    .await
    .expect("SHOW CONFIG after HTTP PUT failed");
    assert_eq!(
        rows(&r)[0]["precisions"].as_str().unwrap(),
        "(11, 10, 9, 8, 7, 6, 4, 2)",
        "a config written over HTTP must be the same record SQL reads"
    );

    let health_http: Value = http_get(
        &base,
        &format!(
            "/api/admin/management/database/{}/{}/spatial/health?workspace={}&property=location",
            TENANT, REPO, WORKSPACE
        ),
        &token,
    )
    .await
    .expect("GET spatial/health failed");
    // `PUT …/spatial/config` queues its own rebuild when the new policy differs
    // from what the local index was built under — a read-mostly workspace would
    // otherwise wait forever for a write to notice the drift. So the drift this
    // PUT created may already be closed by the time health is read back, and
    // asserting `needs_rebuild == true` here is a race, not a contract.
    //
    // The contract is that health tells the TRUTH: it reports a rebuild is owed
    // exactly when intent and reality actually differ.
    assert!(
        put["rebuild_job_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "a config change that drifts from the local index must queue a rebuild: {}",
        put
    );
    let drifted = health_http[0]["indexed_policy_hash"] != health_http[0]["configured_policy_hash"];
    assert_eq!(
        health_http[0]["needs_rebuild"],
        json!(drifted),
        "HTTP health must report a rebuild is owed exactly when the built policy \
         differs from the configured one: {}",
        health_http[0]
    );

    let job: Value = http_post(
        &base,
        &format!(
            "/api/admin/management/database/{}/{}/spatial/rebuild",
            TENANT, REPO
        ),
        &token,
        json!({ "workspace": WORKSPACE, "property": "location" }),
    )
    .await
    .expect("POST spatial/rebuild failed");
    println!("[http rebuild] {}", job);
    assert!(job["job_id"].as_str().is_some_and(|s| !s.is_empty()));

    let row = await_precisions(
        &base,
        REPO,
        &token,
        &DEFAULT_PRECISIONS,
        "HTTP rebuild grows the set back",
    )
    .await;
    assert_eq!(
        row["live_entries"].as_i64().unwrap(),
        (PLACES.len() * DEFAULT_PRECISIONS.len()) as i64,
        "growing the policy must re-create the finer entries for data written long before"
    );
    assert!(
        precisions_present(&row).contains(&11),
        "sub-metre precision must be back for pre-existing rows"
    );
    assert_eq!(
        radius_query(&base, REPO, &token, 5_000).await,
        baseline_near
    );
    assert_eq!(
        radius_query(&base, REPO, &token, 400_000).await,
        baseline_wide
    );

    let verify_http: Value = http_post(
        &base,
        &format!(
            "/api/admin/management/database/{}/{}/spatial/verify",
            TENANT, REPO
        ),
        &token,
        json!({ "workspace": WORKSPACE, "property": "location" }),
    )
    .await
    .expect("POST spatial/verify failed");
    assert_eq!(
        verify_http[0]["status"].as_str().unwrap(),
        "OK",
        "{}",
        verify_http
    );
    println!("[OK] HTTP config → rebuild → verify round trip");

    println!("\n=== spatial admin + reindex e2e PASSED ===\n");
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore] // starts a real server; run with --ignored
async fn spatial_admin_denies_non_admin_callers() {
    const REPO: &str = "spatial_authz_test";
    let (server, admin_token) = boot(8232, REPO).await;
    let base = server.base_url.clone();
    println!("\n=== spatial admin authorization ===\n");

    insert_places(&base, REPO, &admin_token).await;

    // A registered identity user: authenticated, but only viewer/authenticated_user.
    let registered: Value = http_post(
        &base,
        &format!("/auth/{}/register", REPO),
        "",
        json!({
            "email": "not-an-admin@example.com",
            "password": "NotAnAdmin12345!",
            "display_name": "Not An Admin"
        }),
    )
    .await
    .expect("identity registration failed");
    let user_token = registered["access_token"]
        .as_str()
        .expect("register response must carry an access token")
        .to_string();

    // -- SQL: a mutating statement is refused. ---------------------------------
    let err = sql(
        &base,
        REPO,
        &user_token,
        &format!(
            "ALTER SPATIAL INDEX FOR '{}' PROPERTY 'location' SET PRECISIONS = (6, 4, 2)",
            WORKSPACE
        ),
    )
    .await
    .expect_err("a non-admin must not be able to ALTER the spatial index");
    assert!(
        err.contains("system_admin") || err.contains("Insufficient permissions"),
        "denial must say why, got: {}",
        err
    );
    println!("[OK] SQL ALTER denied for a non-admin: {}", err);

    for statement in [
        format!("REBUILD SPATIAL INDEX FOR '{}'", WORKSPACE),
        format!("VERIFY SPATIAL INDEX FOR '{}'", WORKSPACE),
    ] {
        let err = sql(&base, REPO, &user_token, &statement)
            .await
            .unwrap_err_or_panic(&statement);
        assert!(
            err.contains("system_admin") || err.contains("Insufficient permissions"),
            "{} must be denied with a reason, got: {}",
            statement,
            err
        );
    }
    println!("[OK] SQL REBUILD and VERIFY denied for a non-admin");

    // The denial must be a permission decision, not a blanket refusal: a read-only
    // SHOW is still allowed to an authenticated caller.
    let shown = sql(
        &base,
        REPO,
        &user_token,
        &format!("SHOW SPATIAL INDEX CONFIG FOR '{}'", WORKSPACE),
    )
    .await
    .expect("SHOW SPATIAL INDEX CONFIG must remain available to an authenticated user");
    assert!(!rows(&shown).is_empty());
    println!("[OK] read-only SHOW still permitted for a non-admin");

    // -- HTTP: the admin endpoints reject both anonymous and non-admin callers. --
    let cfg_path = format!(
        "/api/admin/management/database/{}/{}/spatial/config",
        TENANT, REPO
    );
    let rebuild_path = format!(
        "/api/admin/management/database/{}/{}/spatial/rebuild",
        TENANT, REPO
    );
    let body = json!({ "workspace": WORKSPACE, "property": "location", "precisions": [6, 4, 2] });

    assert_eq!(
        status_of(reqwest::Method::GET, &base, &cfg_path, None, None).await,
        StatusCode::UNAUTHORIZED,
        "no bearer token must be 401"
    );
    assert_eq!(
        status_of(
            reqwest::Method::PUT,
            &base,
            &cfg_path,
            Some(&user_token),
            Some(body.clone())
        )
        .await,
        StatusCode::FORBIDDEN,
        "an identity user token must be 403 on an admin endpoint"
    );
    assert_eq!(
        status_of(
            reqwest::Method::POST,
            &base,
            &rebuild_path,
            Some(&user_token),
            Some(json!({ "workspace": WORKSPACE }))
        )
        .await,
        StatusCode::FORBIDDEN,
        "an identity user token must not be able to queue a rebuild"
    );
    assert_eq!(
        status_of(
            reqwest::Method::GET,
            &base,
            &cfg_path,
            Some("not-a-real-token"),
            None
        )
        .await,
        StatusCode::UNAUTHORIZED,
        "a garbage bearer must be 401"
    );

    // ...and the admin still gets through, so the gate is not simply broken.
    assert!(
        status_of(
            reqwest::Method::PUT,
            &base,
            &cfg_path,
            Some(&admin_token),
            Some(body)
        )
        .await
        .is_success(),
        "the tenant admin must still be able to write the config"
    );
    println!("[OK] HTTP admin endpoints gated: 401 anonymous, 403 non-admin, 2xx admin");

    println!("\n=== spatial admin authorization PASSED ===\n");
}

/// `Result::unwrap_err` with a message naming the statement.
trait UnwrapErrOrPanic {
    fn unwrap_err_or_panic(self, what: &str) -> String;
}

impl UnwrapErrOrPanic for Result<Value, String> {
    fn unwrap_err_or_panic(self, what: &str) -> String {
        match self {
            Err(e) => e,
            Ok(v) => panic!("{} unexpectedly succeeded: {}", what, v),
        }
    }
}
