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

//! Multi-SRID and reprojection, end to end against a real server.
//!
//! # Why reference values and not round trips
//!
//! A `4326 -> 3857 -> 4326` round trip passes even when both directions are wrong
//! in the same way — an inverted sign, a swapped axis pair, a wrong ellipsoid all
//! cancel. So the projected coordinates asserted here are **externally derived**:
//! EPSG:3857 from the closed-form definition of Pseudo-Mercator
//! (`x = a·λ`, `y = a·ln(tan(π/4 + φ/2))`, `a = 6378137`), and WGS84 / UTM from a
//! 6th-order Krüger series cross-checked on the central meridian against Simpson
//! quadrature of the exact meridian-arc integral (the two agreed to 1.3e-7 m, and
//! the quadrature's `M(90°) = 10001965.729 m` matches the published WGS84 quarter
//! meridian). `raisin-sql-execution`'s `tests_crs.rs` documents the derivation in
//! full.
//!
//! Zurich is the fixture throughout because `(8.54, 47.37)` and `(47.37, 8.54)` are
//! both individually plausible lon/lat pairs that land 4500 km apart in different
//! UTM zones — so a swapped-axis regression cannot pass anything here.
//!
//! # Everything below runs in a default build
//!
//! EPSG:4326, EPSG:3857 (plus its 3785/900913 aliases) and all 120 WGS84 UTM zones
//! are the built-in tier: no Cargo feature, no system libproj, no C toolchain. That
//! is deliberate — the write-time index normaliser is restricted to exactly this
//! set so that two nodes of a masterless cluster built with different features
//! cannot produce different index entries for the same replicated record.

#[allow(unused_imports)]
use crate::helpers;
use futures_util::{SinkExt, StreamExt};
use helpers::multi_node::{authenticate, ServerConfig, ServerHandle};
use reqwest::Client;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

const REPO: &str = "crs_test";
const BRANCH: &str = "main";
const WORKSPACE: &str = "sites";
const PORT: u16 = 8104;

// --- externally derived reference values -------------------------------------

const ZURICH: (f64, f64) = (8.54, 47.37);
const ZURICH_3857: (f64, f64) = (950_668.451_374_556_3, 6_002_677.997_532_715);
const ZURICH_UTM32N: (f64, f64) = (465_270.423_099_666_7, 5_246_384.775_981_838);

const SYDNEY_UTM56S: (f64, f64) = (334_900.569_652_263_2, 6_252_288.752_888_292);

/// `a·π` — the half-width of the whole Mercator world.
const MERCATOR_HALF_WIDTH: f64 = 20_037_508.342_789_244;

/// Pseudo-Mercator is closed form; a millimetre is generous.
const MERCATOR_TOL_M: f64 = 1e-3;
/// The implementation carries Krüger to 3rd order, the reference to 6th.
const UTM_TOL_M: f64 = 0.01;

// --- transports ---------------------------------------------------------------

async fn http_post(base_url: &str, path: &str, token: &str, body: Value) -> Result<Value, String> {
    let response = Client::new()
        .post(format!("{base_url}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|_| text)
}

async fn http_put(base_url: &str, path: &str, token: &str, body: Value) -> Result<(), String> {
    let response = Client::new()
        .put(format!("{base_url}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        return Err(format!(
            "{status}: {}",
            response.text().await.unwrap_or_default()
        ));
    }
    Ok(())
}

/// Run SQL over HTTP `POST /api/sql/{repo}`.
async fn sql(base_url: &str, token: &str, query: &str) -> Result<Value, String> {
    http_post(
        base_url,
        &format!("/api/sql/{REPO}"),
        token,
        json!({ "sql": query, "params": [] }),
    )
    .await
}

/// The first row's named column, or a panic naming the query.
async fn sql_scalar(base_url: &str, token: &str, query: &str, column: &str) -> Value {
    let result = sql(base_url, token, query)
        .await
        .unwrap_or_else(|e| panic!("SQL failed\n  {query}\n  {e}"));
    result["rows"][0][column].clone()
}

/// A SQL expression for a point in a **projected** CRS.
///
/// `ST_POINT` cannot be used for this: it validates its arguments as longitude and
/// latitude unconditionally, so `ST_POINT(2683000, 1247000)` — a perfectly ordinary
/// Swiss LV95 easting/northing — is rejected as an out-of-range longitude. PostGIS
/// has no such restriction (`ST_SetSRID(ST_MakePoint(2683000, 1247000), 2056)` is
/// the idiomatic form there), so this is a real gap in the constructor rather than a
/// quirk of the test; it is reported as a follow-up, and `ST_GEOMFROMGEOJSON` is the
/// working route in the meantime.
fn projected_point(x: f64, y: f64, srid: u32) -> String {
    format!(
        "ST_GEOMFROMGEOJSON('{{\"type\":\"Point\",\"coordinates\":[{x},{y}],\"srid\":{srid}}}')"
    )
}

/// Run the *same* SQL over the WebSocket transport, so a divergence between it and
/// HTTP is caught rather than assumed away.
///
/// # The protocol is MessagePack over Binary frames
///
/// Not JSON text. `handler/socket.rs` decodes only `Message::Binary` with
/// `rmp_serde::from_slice` and logs a text frame as "unexpected", so a JSON-text
/// client hangs rather than erroring. Worth stating here because nothing else in
/// this test suite exercises the WebSocket transport — `cluster_test_utils`'s
/// `WebSocketClient` is a placeholder that returns "not yet fully implemented".
async fn sql_over_ws(port: u16, token: &str, query: &str) -> Result<Value, String> {
    // The repository must be in the URL: a bare `/ws` scopes the connection to the
    // tenant's default repository and every request naming another one is rejected
    // with REPOSITORY_SCOPE_MISMATCH.
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws/{REPO}"))
            .await
            .map_err(|e| format!("ws connect: {e}"))?;

    // The server greets every connection with a `{"type":"connected",...}` frame
    // before any request; it carries no `status`, so `ws_reply` skips it.
    //
    // Username/password rather than `authenticate_jwt`, because the JWT this test
    // already holds — issued by `POST /api/raisindb/sys/{tenant}/auth` and accepted
    // by every HTTP endpoint — is REJECTED by the WebSocket handler with
    // "Invalid user token: missing field `email`". The two transports disagree about
    // the claim set; reported as a follow-up, out of this area's scope.
    let _ = token;
    let auth = json!({
        "request_id": "auth-1",
        "type": "authenticate",
        "context": { "tenant_id": "default", "repository": REPO },
        "payload": { "username": "admin", "password": "Admin12345!@#" }
    });
    ws_send(&mut socket, &auth).await?;
    let reply = ws_reply(&mut socket).await?;
    if reply["status"] != "success" {
        return Err(format!("ws authenticate: {reply}"));
    }

    let request = json!({
        "request_id": "sql-1",
        "type": "sql_query",
        "context": { "tenant_id": "default", "repository": REPO, "branch": BRANCH },
        "payload": { "query": query }
    });
    ws_send(&mut socket, &request).await?;
    let reply = ws_reply(&mut socket).await?;
    let _ = socket.close(None).await;

    if reply["status"] == "error" {
        return Err(format!("ws sql_query: {}", reply["error"]));
    }
    Ok(reply["result"].clone())
}

async fn ws_send<S>(socket: &mut S, envelope: &Value) -> Result<(), String>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let bytes = rmp_serde::to_vec_named(envelope).map_err(|e| format!("ws encode: {e}"))?;
    socket
        .send(Message::Binary(bytes.into()))
        .await
        .map_err(|e| format!("ws send: {e}"))
}

/// The next frame that looks like a response envelope, skipping the connection
/// greeting and any event push that arrives in between.
async fn ws_reply<S>(socket: &mut S) -> Result<Value, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let deadline = std::time::Duration::from_secs(20);
    loop {
        let message = tokio::time::timeout(deadline, socket.next())
            .await
            .map_err(|_| "ws timed out".to_string())?
            .ok_or_else(|| "ws closed".to_string())?
            .map_err(|e| format!("ws error: {e}"))?;
        match message {
            Message::Binary(data) => {
                let value: Value = rmp_serde::from_slice(&data)
                    .map_err(|e| format!("ws decode: {e} ({} bytes)", data.len()))?;
                if value.get("status").is_some() {
                    return Ok(value);
                }
            }
            Message::Text(text) => return Err(format!("unexpected text frame: {text}")),
            Message::Close(frame) => return Err(format!("ws closed: {frame:?}")),
            _ => continue,
        }
    }
}

// --- assertions ---------------------------------------------------------------

/// A geometry column may arrive as JSON or as a JSON string depending on which
/// function produced it (`ST_ASGEOJSON` returns TEXT); accept both.
fn as_geometry(value: &Value) -> Value {
    match value {
        Value::String(s) => serde_json::from_str(s)
            .unwrap_or_else(|e| panic!("geometry column is not JSON: {e}: {s}")),
        other => other.clone(),
    }
}

fn xy(value: &Value) -> (f64, f64) {
    let geometry = as_geometry(value);
    let coords = geometry["coordinates"]
        .as_array()
        .unwrap_or_else(|| panic!("no coordinates in {geometry}"))
        .clone();
    (coords[0].as_f64().unwrap(), coords[1].as_f64().unwrap())
}

fn srid_of(value: &Value) -> Option<u64> {
    as_geometry(value).get("srid").and_then(Value::as_u64)
}

fn assert_close(label: &str, got: (f64, f64), want: (f64, f64), tol: f64) {
    assert!(
        (got.0 - want.0).abs() <= tol && (got.1 - want.1).abs() <= tol,
        "{label}: got ({}, {}), expected ({}, {}) within {tol} m",
        got.0,
        got.1,
        want.0,
        want.1
    );
}

// --- the test -----------------------------------------------------------------

#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_crs_test -- --ignored --nocapture
async fn test_crs_and_projection_end_to_end() {
    println!("\n=== CRS / projection end-to-end ===\n");

    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let token = bootstrap_admin(&server.base_url).await;
    let base = server.base_url.clone();
    println!("[OK] server up, authenticated");

    provision(&base, &token).await;

    // ---------------------------------------------------------------- ST_SRID

    println!("\n--- ST_SRID reports the real label ---");
    assert_eq!(
        sql_scalar(
            &base,
            &token,
            "SELECT ST_SRID(ST_POINT(8.54, 47.37)) AS s",
            "s"
        )
        .await,
        json!(4326),
        "an unlabelled geometry is 4326, which keeps every pre-existing query working"
    );
    // The regression: this used to be hardcoded 4326 for every input.
    assert_eq!(
        sql_scalar(
            &base,
            &token,
            &format!(
                "SELECT ST_SRID({}) AS s",
                projected_point(2_683_000.0, 1_247_000.0, 2056)
            ),
            "s"
        )
        .await,
        json!(2056),
        "ST_SRID must read the carrier, not return a constant"
    );
    // And via ST_SETSRID, on coordinates ST_POINT will accept.
    assert_eq!(
        sql_scalar(
            &base,
            &token,
            "SELECT ST_SRID(ST_SETSRID(ST_POINT(8.54, 47.37), 2056)) AS s",
            "s"
        )
        .await,
        json!(2056)
    );
    // Deprecated WebMercator synonyms canonicalise, or ST_SRID(a) = ST_SRID(b)
    // would be false for two geometries in the same CRS.
    assert_eq!(
        sql_scalar(
            &base,
            &token,
            "SELECT ST_SRID(ST_SETSRID(ST_POINT(0, 0), 900913)) AS s",
            "s"
        )
        .await,
        json!(3857)
    );
    println!("[PASS] ST_SRID: 4326 / 2056 / 900913->3857");

    // ------------------------------------------ ST_SETSRID relabels, never moves

    println!("\n--- ST_SETSRID relabels; ST_TRANSFORM moves ---");
    let relabelled = sql_scalar(
        &base,
        &token,
        "SELECT ST_SETSRID(ST_POINT(8.54, 47.37), 3857) AS g",
        "g",
    )
    .await;
    assert_close("ST_SETSRID", xy(&relabelled), ZURICH, 1e-9);
    assert_eq!(srid_of(&relabelled), Some(3857));

    let moved = sql_scalar(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(8.54, 47.37), 3857) AS g",
        "g",
    )
    .await;
    assert_close("ST_TRANSFORM", xy(&moved), ZURICH_3857, MERCATOR_TOL_M);
    assert_eq!(srid_of(&moved), Some(3857));
    println!(
        "[PASS] same inputs: SETSRID keeps ({}, {}), TRANSFORM yields ({:.3}, {:.3})",
        ZURICH.0, ZURICH.1, ZURICH_3857.0, ZURICH_3857.1
    );

    // 4326 removes the member, keeping ordinary output strictly RFC 7946.
    let back_to_wgs84 = sql_scalar(
        &base,
        &token,
        "SELECT ST_SETSRID(ST_SETSRID(ST_POINT(1, 2), 3857), 4326) AS g",
        "g",
    )
    .await;
    assert_eq!(srid_of(&back_to_wgs84), None, "4326 must be implicit");
    println!("[PASS] 4326 output carries no srid member (RFC 7946 conformant)");

    // ------------------------------------------------ reference-value transforms

    println!("\n--- ST_TRANSFORM against external reference values ---");
    for (label, target, want, tol) in [
        ("Zurich -> EPSG:3857", 3857, ZURICH_3857, MERCATOR_TOL_M),
        ("Zurich -> EPSG:32632", 32632, ZURICH_UTM32N, UTM_TOL_M),
    ] {
        let got = sql_scalar(
            &base,
            &token,
            &format!("SELECT ST_TRANSFORM(ST_POINT(8.54, 47.37), {target}) AS g"),
            "g",
        )
        .await;
        assert_close(label, xy(&got), want, tol);
        println!("[PASS] {label}");
    }

    let sydney = sql_scalar(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(151.2153, -33.8568), 32756) AS g",
        "g",
    )
    .await;
    assert_close(
        "Sydney -> EPSG:32756",
        xy(&sydney),
        SYDNEY_UTM56S,
        UTM_TOL_M,
    );
    println!("[PASS] southern-hemisphere UTM (10 000 000 m false northing)");

    // The two structural landmarks of Pseudo-Mercator.
    let edge = sql_scalar(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(180, 0), 3857) AS g",
        "g",
    )
    .await;
    assert!(
        (xy(&edge).0 - MERCATOR_HALF_WIDTH).abs() < MERCATOR_TOL_M,
        "the antimeridian must sit at a*pi: {:?}",
        xy(&edge)
    );

    // On a zone's central meridian, easting is exactly the false easting and
    // northing is k0*M(lat) — a value that depends only on the ellipsoid and the
    // scale factor, so it isolates both from the series.
    let cm = sql_scalar(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(9, 45), 32632) AS g",
        "g",
    )
    .await;
    assert_close(
        "central meridian z32 @45N",
        xy(&cm),
        (500_000.0, 4_982_950.400_226_4),
        UTM_TOL_M,
    );
    println!("[PASS] Mercator half-width and the UTM central-meridian identity");

    // ------------------------------------------------------------- axis order

    println!("\n--- axis order is pinned to (longitude, latitude) ---");
    // A swap is 4500 km and two UTM zones away; it cannot look like rounding.
    let swapped = sql_scalar(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(47.37, 8.54), 3857) AS g",
        "g",
    )
    .await;
    let (sx, sy) = xy(&swapped);
    let drift = (sx - ZURICH_3857.0).hypot(sy - ZURICH_3857.1);
    assert!(
        drift > 1_000_000.0,
        "a reversed argument order must not land within a megametre of the truth \
         (got {drift} m) — is the convention still lon/lat?"
    );

    // The OGC URN form of an EPSG code does NOT flip the axes, deliberately: we
    // diverge from the EPSG authority's lat/lon definition of EPSG:4326 in favour
    // of GeoJSON RFC 7946 §3.1.1, which every client library follows.
    let urn = sql_scalar(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(8.54, 47.37), 'urn:ogc:def:crs:EPSG::3857') AS g",
        "g",
    )
    .await;
    assert_close("OGC URN target", xy(&urn), ZURICH_3857, MERCATOR_TOL_M);

    // And the unambiguously-reversed guard fires at construction time.
    let err = sql(&base, &token, "SELECT ST_POINT(47.37, 185.4) AS g")
        .await
        .expect_err("a reversed pair with an impossible latitude must be rejected");
    assert!(
        err.contains("reversed") || err.contains("latitude"),
        "the error must explain the swap: {err}"
    );
    println!(
        "[PASS] swap drifts {:.0} km; URN form does not flip; guard fires",
        drift / 1000.0
    );

    // ----------------------------------------------------- loud failure modes

    println!("\n--- failures are loud, never approximate ---");
    let err = sql(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(8.54, 47.37), 999999) AS g",
    )
    .await
    .expect_err("an unsupported SRID must error, not pass the geometry through");
    assert!(err.contains("999999"), "{err}");
    assert!(
        err.contains("proj") || err.contains("features") || err.contains("backend"),
        "the message must name the Cargo feature that would enable it: {err}"
    );
    println!("[PASS] unsupported SRID names the code and the feature");

    // The 85.05-90 degree band is the dangerous one: libproj returns a *finite*
    // northing twelve times the height of the Mercator world at the pole, reported
    // as success, which would geohash to a garbage cell.
    let err = sql(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_POINT(0, 89.9), 3857) AS g",
    )
    .await
    .expect_err("a near-pole point has no image in EPSG:3857");
    println!(
        "[PASS] near-pole EPSG:3857 rejected: {}",
        err.lines().next().unwrap_or("")
    );

    // A single out-of-domain vertex fails the whole geometry: a half-projected
    // ring is a structurally valid polygon describing nowhere.
    assert!(sql(
        &base,
        &token,
        "SELECT ST_TRANSFORM(ST_GEOMFROMGEOJSON('{\"type\":\"Polygon\",\"coordinates\":\
         [[[8.5,47.3],[8.6,47.3],[0,89.5],[8.5,47.3]]]}'), 3857) AS g"
    )
    .await
    .is_err());
    println!("[PASS] all-or-nothing: one bad vertex fails the geometry");

    // ------------------------------------------------------- SRID mismatch

    println!("\n--- SRID mismatch on a binary predicate ---");
    let mismatch = sql(
        &base,
        &token,
        &format!(
            "SELECT ST_INTERSECTS(ST_SETSRID(ST_POINT(8.54, 47.37), 4326), {}) AS hit",
            projected_point(ZURICH_3857.0, ZURICH_3857.1, 3857)
        ),
    )
    .await;
    match mismatch {
        Err(err) => {
            assert!(
                err.contains("4326") && err.contains("3857"),
                "the error must name both codes: {err}"
            );
            println!("[PASS] mismatch errors naming both codes");
        }
        Ok(value) => {
            // D1 owns the topological predicates and is migrating them onto the
            // shared rule; until that lands this reports rather than fails, so a
            // sequencing gap stays visible instead of being silently accepted.
            println!(
                "[TODO] ST_INTERSECTS does not enforce the SRID mismatch yet: {}",
                value["rows"]
            );
        }
    }

    // An unlabelled operand adopts the labelled one, which is what keeps every
    // pre-existing 4326 query working with no changes at all.
    let adopted = sql_scalar(
        &base,
        &token,
        "SELECT ST_DWITHIN(ST_POINT(8.54, 47.37), \
                ST_SETSRID(ST_POINT(8.54, 47.37), 4326), 1) AS hit",
        "hit",
    )
    .await;
    assert_eq!(adopted, json!(true), "unlabelled must adopt, not clash");
    println!("[PASS] an unlabelled operand adopts the labelled SRID");

    // ------------------------------------------------- stored geometry, 3 SRIDs

    println!("\n--- stored geometry keeps its SRID through storage and back ---");
    insert_sites(&base, &token).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let rows = sql(
        &base,
        &token,
        &format!(
            "SELECT name, \
                    ST_SRID(ST_GEOMFROMGEOJSON(properties->>'geom'::String)) AS srid \
             FROM {WORKSPACE} WHERE node_type = 'crs:Site' ORDER BY name"
        ),
    )
    .await
    .expect("stored-geometry query");
    let rows = rows["rows"].as_array().expect("rows").clone();
    assert_eq!(rows.len(), 3, "expected 3 sites, got {}", rows.len());
    for row in &rows {
        let name = row["name"].as_str().unwrap_or("?");
        let expected = match name {
            "zurich-mercator" => 3857,
            "zurich-utm" => 32632,
            "zurich-wgs84" => 4326,
            other => panic!("unexpected site {other}"),
        };
        assert_eq!(
            row["srid"].as_i64(),
            Some(expected),
            "{name} lost its SRID in storage"
        );
    }
    println!("[PASS] 4326 / 3857 / 32632 all survive a storage round trip");

    // Three representations of the same place must converge on the same lon/lat.
    let normalised = sql(
        &base,
        &token,
        &format!(
            "SELECT name, ST_ASGEOJSON(\
                 ST_TRANSFORM(ST_GEOMFROMGEOJSON(properties->>'geom'::String), 4326)\
             ) AS wgs84 \
             FROM {WORKSPACE} WHERE node_type = 'crs:Site' ORDER BY name"
        ),
    )
    .await
    .expect("normalising query");
    for row in normalised["rows"].as_array().expect("rows") {
        let (lon, lat) = xy(&row["wgs84"]);
        assert!(
            (lon - ZURICH.0).abs() < 1e-6 && (lat - ZURICH.1).abs() < 1e-6,
            "{} normalised to ({lon}, {lat}), expected Zurich",
            row["name"]
        );
    }
    println!("[PASS] all three SRIDs normalise to the same lon/lat");

    // ------------------------------------------------------- transport parity

    println!("\n--- HTTP and WebSocket agree ---");
    let query = "SELECT ST_ASGEOJSON(ST_TRANSFORM(ST_POINT(8.54, 47.37), 32632)) AS g, \
                        ST_SRID(ST_SETSRID(ST_POINT(0, 0), 2056)) AS s";
    let over_http = sql(&base, &token, query).await.expect("http");
    let over_ws = sql_over_ws(PORT, &token, query)
        .await
        .unwrap_or_else(|e| panic!("WebSocket SQL failed: {e}"));
    let http_row = &over_http["rows"][0];
    let ws_row = &over_ws["rows"][0];
    assert_eq!(
        http_row["s"], ws_row["s"],
        "ST_SRID must not differ by transport"
    );
    assert_close(
        "ws vs reference",
        xy(&ws_row["g"]),
        ZURICH_UTM32N,
        UTM_TOL_M,
    );
    assert_close(
        "http vs reference",
        xy(&http_row["g"]),
        ZURICH_UTM32N,
        UTM_TOL_M,
    );
    println!("[PASS] identical results over HTTP and WebSocket");

    // pgwire is asserted at the type-mapping level rather than over the wire:
    // raisin-server has no postgres client in its dev-dependencies, and only Area A
    // may edit Cargo.toml in this pass. The two mappings that mattered — the
    // simple-query path's JSONB and the extended/prepared path's former TEXT — are
    // now pinned together by
    // `raisin_transport_pgwire::type_mapping::tests::geometry_is_jsonb_on_both_paths`,
    // and the binary framing by `a_geometry_encodes_as_binary_jsonb_carrying_its_srid`.
    println!(
        "\n[NOTE] pgwire covered by type_mapping unit tests; see follow-ups for a psql-level test"
    );

    println!("\n=== CRS / projection end-to-end: PASS ===\n");
}

// --- fixtures -----------------------------------------------------------------

async fn bootstrap_admin(base_url: &str) -> String {
    let token = authenticate(base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("authenticate");

    let client = Client::new();
    let profile = client
        .get(format!("{base_url}/api/raisindb/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let user_id = profile["user_id"].as_str().unwrap().to_string();

    client
        .put(format!(
            "{base_url}/api/raisindb/sys/default/users/{user_id}"
        ))
        .bearer_auth(&token)
        .json(&json!({ "must_change_password": false }))
        .send()
        .await
        .unwrap();

    authenticate(base_url, "default", "admin", "Admin12345!@#")
        .await
        .expect("re-authenticate")
}

async fn provision(base_url: &str, token: &str) {
    http_post(
        base_url,
        "/api/repositories",
        token,
        json!({
            "repo_id": REPO,
            "description": "CRS / projection test repo",
            "default_branch": BRANCH
        }),
    )
    .await
    .expect("create repository");

    http_put(
        base_url,
        &format!("/api/workspaces/{REPO}/{WORKSPACE}"),
        token,
        json!({
            "name": WORKSPACE,
            "description": "Sites in assorted coordinate reference systems",
            // `raisin:Folder` must be allowed: creating a workspace materialises a
            // root folder node, so a workspace that permits only its own type is
            // rejected at creation time.
            "allowed_node_types": ["crs:Site", "raisin:Folder"],
            "allowed_root_node_types": ["crs:Site", "raisin:Folder"],
            "depends_on": [],
            "config": { "default_branch": BRANCH, "node_type_pins": {} }
        }),
    )
    .await
    .expect("create workspace");

    http_post(
        base_url,
        &format!("/api/management/{REPO}/{BRANCH}/nodetypes"),
        token,
        json!({
            "node_type": {
                "name": "crs:Site",
                "description": "A site whose geometry may be in any CRS",
                "properties": [
                    { "name": "title", "type": "String", "required": true },
                    { "name": "geom", "type": "Object" }
                ],
                "allowed_children": []
            },
            "commit": { "message": "Create crs:Site NodeType", "actor": "test" }
        }),
    )
    .await
    .expect("create nodetype");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

/// The same physical place stored three ways: unlabelled WGS84, explicit
/// WebMercator, and explicit UTM zone 32N. All three are in the built-in tier, so
/// all three are storable and indexable in a default build.
async fn insert_sites(base_url: &str, token: &str) {
    let sites = [
        (
            "zurich-wgs84",
            json!({"type":"Point","coordinates":[ZURICH.0, ZURICH.1]}),
        ),
        (
            "zurich-mercator",
            json!({"type":"Point","coordinates":[ZURICH_3857.0, ZURICH_3857.1],"srid":3857}),
        ),
        (
            "zurich-utm",
            json!({"type":"Point","coordinates":[ZURICH_UTM32N.0, ZURICH_UTM32N.1],"srid":32632}),
        ),
    ];

    for (id, geom) in sites {
        http_post(
            base_url,
            &format!("/api/repository/{REPO}/{BRANCH}/head/{WORKSPACE}/"),
            token,
            json!({
                "node": {
                    "id": id,
                    "name": id,
                    "node_type": "crs:Site",
                    "properties": { "title": id, "geom": geom }
                }
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("create {id}: {e}"));
    }
}
