// Full-stack geospatial integration test
//
// Tests: create repo → workspace → nodetype → insert nodes with GeoJSON → ST_* SQL queries

mod helpers;

use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};

const REPO: &str = "geo_test";
const BRANCH: &str = "main";
const WORKSPACE: &str = "stores";

async fn http_post(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let client = Client::new();
    let response = client
        .post(&format!("{}{}", base_url, path))
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
        .put(&format!("{}{}", base_url, path))
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

async fn execute_sql(
    base_url: &str,
    token: &str,
    sql: &str,
    params: Vec<Value>,
) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{}", REPO),
        token,
        json!({ "sql": sql, "params": params }),
    )
    .await
}

#[tokio::test]
#[ignore] // cargo test --package raisin-server --test geospatial_test -- --ignored --nocapture
async fn test_geospatial_queries() {
    println!("\n=== Geospatial Integration Test ===\n");

    // 1. Start server
    let config = ServerConfig::new(8089);
    let server = ServerHandle::start(config)
        .await
        .expect("Failed to start server");

    // Wait for async admin user creation
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("Failed to authenticate");

    // Clear must_change_password by updating the admin user
    let client = Client::new();
    let profile = client
        .get(&format!("{}/api/raisindb/me", server.base_url))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap();

    client
        .put(&format!(
            "{}/api/raisindb/sys/default/users/{}",
            server.base_url, user_id
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await
        .unwrap();

    // Re-authenticate to get a clean token
    let token = authenticate(&server.base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("Failed to re-authenticate");
    println!("[OK] Server started, authenticated");

    // 2. Create repository
    http_post(
        &server.base_url,
        "/api/repositories",
        &token,
        json!({
            "repo_id": REPO,
            "description": "Geospatial test repo",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("Failed to create repository");
    println!("[OK] Repository '{}' created", REPO);

    // 3. Create workspace
    http_put(
        &server.base_url,
        &format!("/api/workspaces/{}/{}", REPO, WORKSPACE),
        &token,
        json!({
            "name": WORKSPACE,
            "description": "Store locations workspace",
            "allowed_node_types": ["geo:Store"],
            "allowed_root_node_types": ["geo:Store"],
            "depends_on": [],
            "config": {
                "default_branch": BRANCH,
                "node_type_pins": {}
            }
        }),
    )
    .await
    .expect("Failed to create workspace");
    println!("[OK] Workspace '{}' created", WORKSPACE);

    // 4. Create NodeType with location property
    http_post(
        &server.base_url,
        &format!("/api/management/{}/{}/nodetypes", REPO, BRANCH),
        &token,
        json!({
            "node_type": {
                "name": "geo:Store",
                "description": "A store with a location",
                "properties": [
                    { "name": "title", "type": "String", "required": true },
                    { "name": "location", "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create geo:Store NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("Failed to create NodeType");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    println!("[OK] NodeType 'geo:Store' created");

    // 5. Insert store nodes with GeoJSON Point locations
    let stores = vec![
        ("sf-coffee", "SF Coffee", -122.4194_f64, 37.7749_f64),
        ("la-coffee", "LA Coffee", -118.2437, 34.0522),
        ("ny-coffee", "NY Coffee", -73.9857, 40.7484),
    ];

    for (id, title, lon, lat) in &stores {
        http_post(
            &server.base_url,
            &format!("/api/repository/{}/{}/head/{}/", REPO, BRANCH, WORKSPACE),
            &token,
            json!({
                "node": {
                    "id": id,
                    "name": id,
                    "node_type": "geo:Store",
                    "properties": {
                        "title": title,
                        "location": { "type": "Point", "coordinates": [lon, lat] }
                    }
                }
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("Failed to create {}: {}", id, e));
        println!("[OK] Created {} at [{}, {}]", title, lon, lat);
    }

    // Give indexing a moment
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // === SQL Function Tests ===

    // 6. ST_DISTANCE (pure computation)
    println!("\n--- ST_DISTANCE ---");
    let r = execute_sql(
        &server.base_url,
        &token,
        "SELECT ST_DISTANCE(ST_POINT(-122.4194, 37.7749), ST_POINT(-118.2437, 34.0522)) AS dist",
        vec![],
    )
    .await
    .expect("ST_DISTANCE failed");
    let dist = r["rows"][0]["dist"].as_f64().unwrap();
    assert!(
        dist > 400_000.0 && dist < 700_000.0,
        "SF-LA ~559km, got {}m",
        dist
    );
    println!("[PASS] SF→LA = {:.0}m", dist);

    // 7. ST_CONTAINS (pure computation)
    println!("--- ST_CONTAINS ---");
    let r = execute_sql(
        &server.base_url,
        &token,
        "SELECT ST_CONTAINS(\
            ST_GEOMFROMGEOJSON('{\"type\":\"Polygon\",\"coordinates\":[[[-123,37],[-122,37],[-122,38],[-123,38],[-123,37]]]}'),\
            ST_POINT(-122.4194, 37.7749)\
        ) AS inside",
        vec![],
    )
    .await
    .expect("ST_CONTAINS failed");
    assert_eq!(r["rows"][0]["inside"], true);
    println!("[PASS] SF point is inside SF polygon");

    // 8. Function chaining: ST_AREA(ST_BUFFER(ST_POINT(...), 1000))
    println!("--- ST_AREA + ST_BUFFER ---");
    let r = execute_sql(
        &server.base_url,
        &token,
        "SELECT ST_AREA(ST_BUFFER(ST_POINT(-122.4194, 37.7749), 1000)) AS area",
        vec![],
    )
    .await
    .expect("ST_AREA/ST_BUFFER failed");
    let area = r["rows"][0]["area"].as_f64().unwrap();
    assert!(area > 0.0, "Buffer area > 0, got {}", area);
    println!("[PASS] Buffer area = {:.0} sq m", area);

    // 9. Accessor functions
    println!("--- Accessor functions ---");
    let r = execute_sql(
        &server.base_url,
        &token,
        "SELECT ST_GEOMETRYTYPE(ST_POINT(0, 0)) AS gtype, \
                ST_NUMPOINTS(ST_POINT(0, 0)) AS npts, \
                ST_SRID(ST_POINT(0, 0)) AS srid",
        vec![],
    )
    .await
    .expect("Accessors failed");
    assert_eq!(r["rows"][0]["gtype"].as_str().unwrap(), "ST_Point");
    assert_eq!(r["rows"][0]["npts"].as_i64().unwrap(), 1);
    assert_eq!(r["rows"][0]["srid"].as_i64().unwrap(), 4326);
    println!("[PASS] ST_GEOMETRYTYPE, ST_NUMPOINTS, ST_SRID");

    // 10. Constructor functions
    println!("--- Constructor functions ---");
    let r = execute_sql(
        &server.base_url,
        &token,
        "SELECT ST_GEOMETRYTYPE(ST_MAKEENVELOPE(-122.5, 37.7, -122.4, 37.8)) AS etype, \
                ST_GEOMETRYTYPE(ST_MAKELINE(ST_POINT(0,0), ST_POINT(1,1))) AS ltype",
        vec![],
    )
    .await
    .expect("Constructors failed");
    assert_eq!(r["rows"][0]["etype"].as_str().unwrap(), "ST_Polygon");
    assert_eq!(r["rows"][0]["ltype"].as_str().unwrap(), "ST_LineString");
    println!("[PASS] ST_MAKEENVELOPE→Polygon, ST_MAKELINE→LineString");

    // 11. Query STORED geometry from node properties (the real test!)
    println!("\n--- Stored geometry query (properties->>'location') ---");
    let r = execute_sql(
        &server.base_url,
        &token,
        &format!(
            "SELECT name, \
                ST_DISTANCE(\
                    ST_GEOMFROMGEOJSON(properties->>'location'::String), \
                    ST_POINT(-122.4194, 37.7749)\
                ) AS dist \
             FROM {} \
             WHERE node_type = 'geo:Store' \
             ORDER BY dist \
             LIMIT 3",
            WORKSPACE
        ),
        vec![],
    )
    .await
    .expect("Stored geometry query failed");

    let rows = r["rows"].as_array().expect("rows");
    println!("  Got {} rows", rows.len());
    for row in rows {
        println!(
            "  {} — {:.0}m",
            row["name"].as_str().unwrap_or("?"),
            row["dist"].as_f64().unwrap_or(0.0)
        );
    }

    assert!(rows.len() >= 3, "Expected 3 stores, got {}", rows.len());

    // SF should be closest (distance ~0)
    let sf_dist = rows[0]["dist"].as_f64().unwrap();
    let la_dist = rows[1]["dist"].as_f64().unwrap();
    let ny_dist = rows[2]["dist"].as_f64().unwrap();

    assert!(sf_dist < 100.0, "SF should be ~0m, got {}", sf_dist);
    assert!(la_dist > 400_000.0, "LA should be >400km, got {}", la_dist);
    assert!(
        ny_dist > 3_500_000.0,
        "NY should be >3500km, got {}",
        ny_dist
    );
    assert!(
        sf_dist < la_dist && la_dist < ny_dist,
        "Should be ordered by distance"
    );
    println!(
        "[PASS] Stored geometry: SF({:.0}m) < LA({:.0}m) < NY({:.0}m)",
        sf_dist, la_dist, ny_dist
    );

    println!("\n=== All geospatial tests passed! ===");
}
