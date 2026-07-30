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

//! **The same geometry SQL over all three transports must give the same answer.**
//!
//! SQL is the interface, and it is reachable three ways — HTTP `POST /api/sql/{repo}`,
//! the WebSocket `sql_query` request, and the PostgreSQL wire protocol. A row written
//! over one must be spatially queryable over the others, and a geometry column must
//! come back as usable GeoJSON on each rather than an opaque blob or a differently
//! stringified shape.
//!
//! # What is genuinely different between the three, and why that is fine
//!
//! * HTTP and WebSocket both return JSON, so a geometry arrives as a JSON object.
//! * pgwire is typed. A geometry column is `JSONB` (`type_mapping.rs`), on the simple
//!   *and* the extended/prepared path — those two disagreed until recently
//!   (`extended_query/schema.rs` said `TEXT`), which meant `psql` decoded the same
//!   bytes differently depending on whether the statement was prepared. Both paths are
//!   exercised below so neither can drift again.
//! * The simple-query protocol has no binary format at all: every value is text. So
//!   pgwire's geometry arrives as a JSON *string* there and as a parsed value on the
//!   extended path. That is the postgres protocol, not a RaisinDB divergence — the test
//!   therefore compares the geometry after parsing, and separately asserts that the
//!   text form parses as JSON at all.
//!
//! Anything beyond that — a different row set, a mangled coordinate, a lost `srid`, a
//! transport that cannot write — is a finding, and the test fails rather than
//! normalising it away.

#[allow(unused_imports)]
use crate::helpers;
use helpers::multi_node::{ServerConfig, ServerHandle};
use helpers::sql_geo::{
    admin_user_id, bootstrap_admin, create_api_key, grant_pgwire_identity, pg_error, pg_wait_ready,
    provision, sql_http, sql_ws,
};
use serde_json::Value;
use std::time::Duration;

const REPO: &str = "parity";
const BRANCH: &str = "main";
const WS: &str = "places";
const NODE_TYPE: &str = "geo:Place";
const HTTP_PORT: u16 = 8107;
const PGWIRE_PORT: u16 = 55_433;

/// One centre, three rows within 200 m of it — one written per transport.
const CENTER: (f64, f64) = (8.5402, 47.3779);

/// `(id, transport label, latitude offset)` — offsets are ~11 m apart, all inside
/// the 200 m radius the parity query uses.
const ROWS: [(&str, &str, f64); 3] = [
    ("p-http", "HTTP", 0.000_10),
    ("p-ws", "WebSocket", 0.000_20),
    ("p-pg", "PGWire", 0.000_30),
];

fn insert_sql(id: &str, label: &str, lat_offset: f64) -> String {
    // `srid` is carried explicitly so a transport that mangles the geometry into a
    // plain string, or drops unknown members, is caught rather than tolerated.
    let lon = CENTER.0;
    let lat = CENTER.1 + lat_offset;
    format!(
        "INSERT INTO '{WS}' (id, path, node_type, properties) VALUES \
         ('{id}', '/{id}', '{NODE_TYPE}', \
          '{{\"title\":\"{label}\",\"location\":\
             {{\"type\":\"Point\",\"coordinates\":[{lon},{lat}],\"srid\":4326}}}}'::JSONB)"
    )
}

/// The parity query: a spatial predicate plus the geometry itself, so both the row
/// set and the value encoding are compared.
fn select_sql() -> String {
    format!(
        "SELECT name, CAST(properties->>'location' AS GEOMETRY) AS geom \
         FROM '{WS}' \
         WHERE ST_DWITHIN(CAST(properties->>'location' AS GEOMETRY), \
                          ST_POINT({}, {}), 200) \
         ORDER BY name",
        CENTER.0, CENTER.1
    )
}

// ------------------------------------------------------- one normalised row shape

/// `(name, geometry)` with the geometry parsed, whatever the transport's encoding.
type Rows = Vec<(String, Value)>;

/// Parse a geometry cell that may arrive as JSON or as a JSON string.
///
/// Both are legitimate: pgwire's simple-query protocol transmits every value as
/// text. What is *not* legitimate is a value that will not parse as GeoJSON at all,
/// which is what "opaque blob" would look like here.
fn parse_geometry(transport: &str, value: &Value) -> Value {
    match value {
        Value::String(text) => serde_json::from_str(text).unwrap_or_else(|e| {
            panic!("{transport}: geometry column is not parseable JSON ({e}): {text}")
        }),
        Value::Null => panic!("{transport}: geometry column came back NULL"),
        other => other.clone(),
    }
}

fn rows_from_json(transport: &str, result: &Value) -> Rows {
    result["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{transport}: no rows array in {result}"))
        .iter()
        .map(|row| {
            let name = row["name"]
                .as_str()
                .unwrap_or_else(|| panic!("{transport}: row without a name: {row}"))
                .to_string();
            (name, parse_geometry(transport, &row["geom"]))
        })
        .collect()
}

fn assert_geometry_is_a_point(transport: &str, name: &str, geom: &Value) {
    assert_eq!(
        geom["type"].as_str(),
        Some("Point"),
        "{transport}/{name}: geometry lost its type: {geom}"
    );
    let coords = geom["coordinates"]
        .as_array()
        .unwrap_or_else(|| panic!("{transport}/{name}: no coordinates: {geom}"));
    assert_eq!(
        coords.len(),
        2,
        "{transport}/{name}: expected a 2-D position: {geom}"
    );
    let lon = coords[0].as_f64().expect("lon is a number");
    let lat = coords[1].as_f64().expect("lat is a number");
    assert!(
        (lon - CENTER.0).abs() < 1e-9,
        "{transport}/{name}: longitude drifted: {lon}"
    );
    assert!(
        (lat - CENTER.1).abs() < 0.001,
        "{transport}/{name}: latitude drifted: {lat}"
    );
    // The explicit SRID must survive the round trip; losing it would silently
    // reinterpret a projected geometry as lon/lat somewhere downstream.
    assert_eq!(
        geom["srid"].as_u64(),
        Some(4326),
        "{transport}/{name}: srid member lost in transit: {geom}"
    );
}

// ------------------------------------------------------------------------- test

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_transport_parity_test -- --ignored --nocapture
async fn test_geometry_sql_parity_across_http_ws_and_pgwire() {
    println!("\n=== geometry SQL parity: HTTP / WebSocket / PGWire ===\n");

    let server = ServerHandle::start(ServerConfig::new(HTTP_PORT).with_pgwire(PGWIRE_PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    let base = server.base_url.clone();
    provision(&base, &token, REPO, BRANCH, WS, NODE_TYPE).await;
    println!("[OK] server up (http {HTTP_PORT}, pgwire {PGWIRE_PORT}), provisioned");

    let api_key = create_api_key(&base, &token).await;
    let pg = pg_wait_ready(PGWIRE_PORT, REPO, &api_key).await;
    println!("[OK] pgwire accepted a real postgres client");

    // An API-key-authenticated pgwire connection carries NO auth context, so it
    // cannot write. `SET app.user` with the genuine admin JWT is the supported way
    // to give the session an identity; see `grant_pgwire_identity` for why the
    // `raisin:User` node has to exist first.
    let user_id = admin_user_id(&base, &token).await;
    grant_pgwire_identity(&base, &token, REPO, &user_id).await;
    pg.simple_query(&format!("SET app.user = '{token}'"))
        .await
        .unwrap_or_else(|e| panic!("SET app.user failed: {}", pg_error(&e)));
    println!("[OK] pgwire session bound to the admin identity");

    // ------------------------------------------------ 1. one write per transport

    println!("\n--- 1. the same INSERT over each transport ---");
    for (id, label, offset) in ROWS {
        let sql = insert_sql(id, label, offset);
        match label {
            "HTTP" => {
                sql_http(&base, &token, REPO, &sql)
                    .await
                    .unwrap_or_else(|e| panic!("HTTP insert failed: {e}"));
            }
            "WebSocket" => {
                sql_ws(HTTP_PORT, REPO, BRANCH, &sql)
                    .await
                    .unwrap_or_else(|e| panic!("WebSocket insert failed: {e}"));
            }
            "PGWire" => {
                pg.simple_query(&sql)
                    .await
                    .unwrap_or_else(|e| panic!("PGWire insert failed: {}", pg_error(&e)));
            }
            other => unreachable!("{other}"),
        }
        println!("[OK] wrote {id} over {label}");
    }
    // Indexing is inline in the write batch, but the WS/pgwire writes are separate
    // requests; give the last one a moment before reading across transports.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // -------------------------------------------- 2. the same SELECT, four ways

    println!("\n--- 2. the same spatial SELECT over each transport ---");
    let query = select_sql();

    let http_rows = rows_from_json(
        "HTTP",
        &sql_http(&base, &token, REPO, &query)
            .await
            .unwrap_or_else(|e| panic!("HTTP select failed: {e}")),
    );
    let ws_rows = rows_from_json(
        "WebSocket",
        &sql_ws(HTTP_PORT, REPO, BRANCH, &query)
            .await
            .unwrap_or_else(|e| panic!("WebSocket select failed: {e}")),
    );
    let pg_simple_rows = pgwire_simple_rows(&pg, &query).await;
    let pg_extended_rows = pgwire_extended_rows(&pg, &query).await;

    for (transport, rows) in [
        ("HTTP", &http_rows),
        ("WebSocket", &ws_rows),
        ("PGWire/simple", &pg_simple_rows),
        ("PGWire/extended", &pg_extended_rows),
    ] {
        println!(
            "  {transport}: {:?}",
            rows.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
        );
    }

    // All three rows must be visible to all three transports. A transport that
    // wrote but cannot read another's row is a replication-of-nothing bug; a
    // transport whose spatial predicate returns fewer rows is the silent-empty bug.
    // `ORDER BY name`, so the expectation is sorted rather than written in the
    // order the rows happened to be inserted.
    let mut expected: Vec<String> = ROWS.iter().map(|(id, _, _)| id.to_string()).collect();
    expected.sort();
    for (transport, rows) in [
        ("HTTP", &http_rows),
        ("WebSocket", &ws_rows),
        ("PGWire/simple", &pg_simple_rows),
        ("PGWire/extended", &pg_extended_rows),
    ] {
        let names: Vec<String> = rows.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(
            names, expected,
            "{transport} returned a different row set for the same spatial SELECT"
        );
        for (name, geom) in rows.iter() {
            assert_geometry_is_a_point(transport, name, geom);
        }
    }
    println!("[PASS] identical row sets, and every geometry is usable GeoJSON with its srid");

    // Byte-for-byte agreement on the parsed geometry, not merely "both are points".
    assert_eq!(
        http_rows, ws_rows,
        "HTTP and WebSocket disagree on the same rows"
    );
    assert_eq!(
        http_rows, pg_simple_rows,
        "HTTP and PGWire (simple query) disagree on the same rows"
    );
    assert_eq!(
        pg_simple_rows, pg_extended_rows,
        "PGWire's simple and extended/prepared paths disagree — the TEXT/JSONB \
         split that used to exist between type_mapping.rs and extended_query/schema.rs"
    );
    println!("[PASS] all four paths agree on the parsed geometry value");

    // ------------------------------------- 3. UPDATE over pgwire is index-visible

    println!("\n--- 3. an UPDATE over pgwire moves the index entry for everyone ---");
    let moved = (8.6, 47.6);
    pg.simple_query(&format!(
        "UPDATE '{WS}' SET properties = \
         '{{\"title\":\"moved\",\"location\":{{\"type\":\"Point\",\
            \"coordinates\":[{}, {}],\"srid\":4326}}}}'::JSONB \
         WHERE id = 'p-pg'",
        moved.0, moved.1
    ))
    .await
    .unwrap_or_else(|e| panic!("PGWire update failed: {}", pg_error(&e)));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let after = rows_from_json(
        "HTTP",
        &sql_http(&base, &token, REPO, &query)
            .await
            .expect("HTTP select after pgwire update"),
    );
    let names: Vec<String> = after.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(
        names,
        vec!["p-http".to_string(), "p-ws".to_string()],
        "a pgwire UPDATE must remove the old spatial index entry, visibly to HTTP"
    );

    let at_new = sql_http(
        &base,
        &token,
        REPO,
        &format!(
            "SELECT name FROM '{WS}' WHERE ST_DWITHIN(\
                 CAST(properties->>'location' AS GEOMETRY), ST_POINT({}, {}), 200)",
            moved.0, moved.1
        ),
    )
    .await
    .expect("HTTP select at the new location");
    assert_eq!(
        at_new["rows"][0]["name"].as_str(),
        Some("p-pg"),
        "the row must match at its new location: {}",
        at_new["rows"]
    );
    println!("[PASS] a pgwire write is index-visible to HTTP, at the new cell and not the old");

    // --------------------------------------- 4. DELETE over WebSocket, same check

    println!("\n--- 4. a DELETE over WebSocket is index-visible to pgwire ---");
    sql_ws(
        HTTP_PORT,
        REPO,
        BRANCH,
        &format!("DELETE FROM '{WS}' WHERE id = 'p-ws'"),
    )
    .await
    .unwrap_or_else(|e| panic!("WebSocket delete failed: {e}"));
    tokio::time::sleep(Duration::from_millis(500)).await;

    let remaining = pgwire_simple_rows(&pg, &query).await;
    let names: Vec<String> = remaining.iter().map(|(n, _)| n.clone()).collect();
    assert_eq!(
        names,
        vec!["p-http".to_string()],
        "a WebSocket DELETE must remove the spatial index entry, visibly to pgwire"
    );
    println!("[PASS] a WebSocket delete is index-visible to pgwire");

    println!("\n=== geometry SQL parity: PASS ===\n");
}

// ------------------------------------------------------------- pgwire plumbing

/// Simple-query protocol: every value arrives as text (postgres has no binary
/// format there), so the geometry is a JSON string that must still parse.
async fn pgwire_simple_rows(client: &tokio_postgres::Client, query: &str) -> Rows {
    use tokio_postgres::SimpleQueryMessage;
    let messages = client
        .simple_query(query)
        .await
        .unwrap_or_else(|e| panic!("PGWire simple_query failed: {}", pg_error(&e)));
    let mut out = Rows::new();
    for message in messages {
        if let SimpleQueryMessage::Row(row) = message {
            let name = row
                .try_get("name")
                .ok()
                .flatten()
                .unwrap_or_else(|| panic!("PGWire/simple: no name column"))
                .to_string();
            let geom = row
                .try_get("geom")
                .ok()
                .flatten()
                .unwrap_or_else(|| panic!("PGWire/simple: no geom column"))
                .to_string();
            out.push((name, parse_geometry("PGWire/simple", &Value::String(geom))));
        }
    }
    out
}

/// Extended/prepared protocol: the geometry column is described as `JSONB`, so
/// `tokio-postgres` decodes it into a `serde_json::Value` with no help from us.
/// That is the assertion — a column described as `TEXT` here (as it was until
/// recently) would make this `get` fail outright.
async fn pgwire_extended_rows(client: &tokio_postgres::Client, query: &str) -> Rows {
    let rows = client
        .query(query, &[])
        .await
        .unwrap_or_else(|e| panic!("PGWire extended query failed: {}", pg_error(&e)));
    rows.iter()
        .map(|row| {
            let name: String = row.get("name");
            let geom: Value = row.get("geom");
            (name, parse_geometry("PGWire/extended", &geom))
        })
        .collect()
}
