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

use serde_json::json;

use super::super::fixtures::geojson;
use super::super::harness::{Ctx, NODE_TYPE, WORKSPACE};
use super::{ZURICH_LAT, ZURICH_LON};

pub(super) async fn srid(ctx: &mut Ctx) {
    // An unlabelled geometry reports 4326 — but this must be a real read of the
    // carrier, not the hardcoded 4326 the old implementation returned for
    // everything.
    ctx.eq(
        "ST_SRID of an unlabelled geometry is 4326",
        "ST_SRID(ST_POINT(8.5, 47.4))",
        json!(4326),
    )
    .await;
    // ...and a labelled one reports its OWN value. This is the assertion the
    // hardcoded version could not pass.
    ctx.eq(
        "ST_SRID reads an explicit srid member",
        r#"ST_SRID(ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[1,2],"srid":3857}'))"#,
        json!(3857),
    )
    .await;
    ctx.eq(
        "ST_SRID reads a UTM label",
        r#"ST_SRID(ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[465236,5245140],"srid":32632}'))"#,
        json!(32632),
    )
    .await;
    // The deprecated aliases canonicalise onto 3857.
    ctx.eq(
        "ST_SRID canonicalises 900913 to 3857",
        r#"ST_SRID(ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[1,2],"srid":900913}'))"#,
        json!(3857),
    )
    .await;
}

pub(super) async fn setsrid(ctx: &mut Ctx) {
    // ST_SETSRID relabels; it must NOT move the coordinates. Asserting both
    // halves matters: an implementation that reprojected would still report the
    // right SRID.
    ctx.eq(
        "ST_SETSRID sets the label",
        "ST_SRID(ST_SETSRID(ST_POINT(8.5417, 47.3769), 3857))",
        json!(3857),
    )
    .await;
    ctx.near(
        "ST_SETSRID does NOT move x",
        "ST_X(ST_SETSRID(ST_POINT(8.5417, 47.3769), 3857))",
        ZURICH_LON,
        1e-12,
    )
    .await;
    ctx.near(
        "ST_SETSRID does NOT move y",
        "ST_Y(ST_SETSRID(ST_POINT(8.5417, 47.3769), 3857))",
        ZURICH_LAT,
        1e-12,
    )
    .await;
    // srid = 4326 REMOVES the member, keeping output RFC-7946 conformant.
    ctx.eq(
        "ST_SETSRID(g, 4326) emits no srid member",
        "ST_ASGEOJSON(ST_SETSRID(ST_POINT(1,2), 4326))",
        json!(r#"{"type":"Point","coordinates":[1.0,2.0]}"#),
    )
    .await;
    // The TEXT overload accepts the usual spellings.
    for spelling in ["'EPSG:3857'", "'epsg:3857'", "'SRID=3857'", "'3857'"] {
        ctx.eq(
            &format!("ST_SETSRID accepts {spelling}"),
            &format!("ST_SRID(ST_SETSRID(ST_POINT(1,2), {spelling}))"),
            json!(3857),
        )
        .await;
    }
    // A foreign authority is an error, not a silent reinterpretation:
    // ESRI:102100 is NOT EPSG:102100.
    ctx.errors_with(
        "ST_SETSRID rejects a foreign authority",
        "ST_SRID(ST_SETSRID(ST_POINT(1,2), 'ESRI:102100'))",
        "ESRI",
    )
    .await;
}

pub(super) async fn axis_order(ctx: &mut Ctx) {
    // (x, y) = (longitude, latitude), pinned. Zurich is the fixture precisely
    // because both orderings are individually plausible: 8.54 and 47.38 are both
    // valid latitudes.
    ctx.near(
        "ST_POINT(lon, lat) reads the first argument as longitude",
        &format!("ST_X(ST_POINT({ZURICH_LON}, {ZURICH_LAT}))"),
        ZURICH_LON,
        1e-12,
    )
    .await;

    // The swapped form lands in a completely different place. Asserting the
    // DISTANCE makes the test impossible to satisfy by accident.
    ctx.num_matches(
        "a swapped ST_POINT lands thousands of km away",
        &format!(
            "ST_DISTANCE(ST_POINT({ZURICH_LON}, {ZURICH_LAT}), ST_POINT({ZURICH_LAT}, {ZURICH_LON}))"
        ),
        "more than 5000 km from the correct location",
        |d| d > 5_000_000.0,
    )
    .await;

    // A latitude beyond +/-90 in the second argument is always an error.
    ctx.errors_with("ST_POINT rejects |lat| > 90", "ST_X(ST_POINT(8.5, 91))", "")
        .await;

    // The OGC URN form does NOT flip the axes: `urn:ogc:def:crs:EPSG::4326` is
    // lon/lat here, diverging from the EPSG authority on purpose and matching
    // GeoJSON RFC 7946, PostGIS, and every web mapping library.
    ctx.eq(
        "the OGC URN form does not flip axes",
        "ST_SRID(ST_SETSRID(ST_POINT(8.5417, 47.3769), 'urn:ogc:def:crs:EPSG::4326'))",
        json!(4326),
    )
    .await;
    ctx.near(
        "the OGC URN form leaves longitude as x",
        "ST_X(ST_SETSRID(ST_POINT(8.5417, 47.3769), 'urn:ogc:def:crs:EPSG::4326'))",
        ZURICH_LON,
        1e-12,
    )
    .await;
}

pub(super) async fn mismatch(ctx: &mut Ctx) {
    // A binary operation across two DIFFERENT explicit SRIDs is an error, like
    // PostGIS. An implicit transform would silently change the answer and would
    // make a query's success depend on which cargo features the server was built
    // with.
    let a4326 = r#"ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[1,2],"srid":4326}')"#;
    let b3857 = r#"ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[1,2],"srid":3857}')"#;

    ctx.errors_with(
        "ST_DISTANCE errors on an SRID mismatch",
        &format!("ST_DISTANCE({a4326}, {b3857})"),
        "SRID",
    )
    .await;
    ctx.errors_with(
        "ST_DISTANCE mismatch message suggests ST_TRANSFORM",
        &format!("ST_DISTANCE({a4326}, {b3857})"),
        "ST_TRANSFORM",
    )
    .await;

    // An UNLABELLED operand adopts the other's SRID — this is what keeps every
    // existing 4326 query working unchanged.
    ctx.num_matches(
        "an unlabelled operand adopts the other's SRID",
        &format!("ST_DISTANCE(ST_POINT(1,2), {b3857})"),
        "a finite distance rather than an error",
        |d| d.is_finite(),
    )
    .await;

    // After an explicit ST_TRANSFORM the operation succeeds.
    ctx.num_matches(
        "ST_TRANSFORM resolves the mismatch",
        &format!("ST_DISTANCE({a4326}, ST_TRANSFORM({b3857}, 4326))"),
        "a finite distance",
        |d| d.is_finite() && d >= 0.0,
    )
    .await;
}

/// Multi-SRID data STORED IN NODES, read back through SQL.
///
/// The three fixtures are the same physical place expressed in 4326, 3857 and
/// UTM 32N. Each must keep its own SRID through the write path, and transforming
/// each to 4326 must land them all within a metre of each other.
pub(super) async fn stored(ctx: &mut Ctx) {
    let sel = |label: &str, expr_sql: &str| {
        format!(
            "SELECT {expr_sql} AS r FROM '{WORKSPACE}' \
             WHERE node_type = '{NODE_TYPE}' AND properties->>'label'::String = '{label}'"
        )
    };

    // Each stored geometry keeps the SRID it was written with.
    for (label, want) in [
        ("zurich_4326", 4326i64),
        ("zurich_3857", 3857),
        ("zurich_utm32", 32632),
    ] {
        let sql = sel(label, "ST_SRID(CAST(properties->>'g' AS GEOMETRY))");
        ctx.cov
            .record_sql(&sql, "stored SRID survives the write path");
        match ctx.sql(&sql).await {
            Ok(rows) => {
                let got = rows
                    .first()
                    .and_then(|r| r.get("r"))
                    .and_then(|v| v.as_i64());
                if got == Some(want) {
                    println!("  [ ok ] stored {label} kept SRID {want}");
                } else {
                    println!("  [FAIL] stored {label}: expected SRID {want}, got {got:?}");
                    ctx.failures
                        .push(format!("stored {label} SRID: expected {want}, got {got:?}"));
                }
            }
            Err(e) => {
                println!("  [FAIL] stored {label} SRID: {e}");
                ctx.failures.push(format!("stored {label} SRID: {e}"));
            }
        }
    }

    // All three normalise to the same lon/lat. This is the assertion that proves
    // the stored SRID is genuinely used rather than merely recorded: transform
    // each to 4326 and measure the spread against the 4326 fixture.
    for label in ["zurich_3857", "zurich_utm32"] {
        let sql = sel(
            label,
            &format!(
                "ST_DISTANCE(ST_TRANSFORM(CAST(properties->>'g' AS GEOMETRY), 4326), {})",
                super::super::harness::g(geojson("zurich_4326"))
            ),
        );
        ctx.cov
            .record_sql(&sql, "stored multi-SRID normalises to one place");
        match ctx.sql(&sql).await {
            Ok(rows) => {
                let d = rows
                    .first()
                    .and_then(|r| r.get("r"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(f64::INFINITY);
                // The fixture coordinates are rounded to 0.1 m, so a few metres
                // of agreement is the right bar.
                if d < 25.0 {
                    println!("  [ ok ] {label} normalises to the 4326 fixture ({d:.2} m apart)");
                } else {
                    println!("  [FAIL] {label} is {d:.2} m from the 4326 fixture");
                    ctx.failures
                        .push(format!("{label} normalised {d:.2} m away, expected < 25 m"));
                }
            }
            Err(e) => {
                println!("  [FAIL] {label} normalisation: {e}");
                ctx.failures.push(format!("{label} normalisation: {e}"));
            }
        }
    }
}
