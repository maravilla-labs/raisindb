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

use super::super::fixtures::expr;
use super::super::harness::{g, Ctx};

pub(super) async fn shapes(ctx: &mut Ctx) {
    // ST_CENTROID of a symmetric square is its middle.
    ctx.eq(
        "ST_CENTROID of a square",
        &format!(
            "ST_ASGEOJSON(ST_CENTROID({}))",
            g(r#"{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}"#)
        ),
        json!(r#"{"type":"Point","coordinates":[1.0,1.0]}"#),
    )
    .await;
    for label in ["pt", "ls", "poly_hole", "mpt", "mls", "mpoly", "gc"] {
        ctx.eq(
            &format!("ST_CENTROID accepts {label}"),
            &format!("ST_GEOMETRYTYPE(ST_CENTROID({}))", expr(label)),
            json!("ST_Point"),
        )
        .await;
    }

    // ST_ENVELOPE is the bounding box of the whole geometry.
    // The MultiPoint fixture spans x 0..3, y 0..4.
    ctx.near(
        "ST_ENVELOPE spans the full x range",
        &format!("ST_X(ST_CENTROID(ST_ENVELOPE({}))) ", expr("mpt")),
        1.5,
        1e-9,
    )
    .await;
    ctx.near(
        "ST_ENVELOPE spans the full y range",
        &format!("ST_Y(ST_CENTROID(ST_ENVELOPE({})))", expr("mpt")),
        2.0,
        1e-9,
    )
    .await;
    ctx.eq(
        "ST_ENVELOPE returns a Polygon",
        &format!("ST_GEOMETRYTYPE(ST_ENVELOPE({}))", expr("mpt")),
        json!("ST_Polygon"),
    )
    .await;

    // ST_CONVEXHULL drops an interior point: the hull of a square plus its
    // centre is the square, so 5 ring vertices, not 6.
    ctx.eq(
        "ST_CONVEXHULL discards interior points",
        &format!(
            "ST_NUMPOINTS(ST_CONVEXHULL({}))",
            g(r#"{"type":"MultiPoint","coordinates":[[0,0],[2,0],[2,2],[0,2],[1,1]]}"#)
        ),
        json!(5),
    )
    .await;
    ctx.is_true(
        "ST_CONVEXHULL covers its input",
        &format!("ST_COVERS(ST_CONVEXHULL({0}), {0})", expr("mpt")),
    )
    .await;

    // ST_REVERSE flips coordinate order and preserves the type.
    ctx.eq(
        "ST_REVERSE reverses a LineString",
        &format!(
            "ST_ASGEOJSON(ST_REVERSE({}))",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,0]]}"#)
        ),
        json!(r#"{"type":"LineString","coordinates":[[2.0,0.0],[1.0,1.0],[0.0,0.0]]}"#),
    )
    .await;
    // Reversing twice is the identity, and a reversed line is topologically equal.
    ctx.is_true(
        "ST_REVERSE twice is the identity",
        &format!("ST_EQUALS(ST_REVERSE(ST_REVERSE({0})), {0})", expr("ls")),
    )
    .await;

    // ST_BOUNDARY: LineString -> its two endpoints; Polygon -> all rings.
    ctx.eq(
        "ST_BOUNDARY of a LineString is its endpoints",
        &format!(
            "ST_ASGEOJSON(ST_BOUNDARY({}))",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,0]]}"#)
        ),
        json!(r#"{"type":"MultiPoint","coordinates":[[0.0,0.0],[2.0,0.0]]}"#),
    )
    .await;
    // A CLOSED LineString has no boundary (documented change).
    ctx.is_true(
        "ST_BOUNDARY of a closed LineString is empty",
        &format!(
            "ST_ISEMPTY(ST_BOUNDARY({}))",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,0],[1,1],[0,0]]}"#)
        ),
    )
    .await;
    // A Polygon's boundary includes INTERIOR rings: shell + hole = 2 lines.
    ctx.eq(
        "ST_BOUNDARY of a holed Polygon includes the interior ring",
        &format!("ST_NUMGEOMETRIES(ST_BOUNDARY({}))", expr("poly_hole")),
        json!(2),
    )
    .await;

    // ST_SIMPLIFY drops a vertex that is within the tolerance of the chord.
    // The tolerance is in METRES on a geographic CRS (documented divergence:
    // PostGIS's geometry type would read this as degrees).
    let nearly_straight = r#"{"type":"LineString","coordinates":[[0,0],[0.0000001,0.5],[0,1]]}"#;
    ctx.eq(
        "ST_SIMPLIFY tolerance is METRES and drops a near-collinear vertex",
        &format!("ST_NUMPOINTS(ST_SIMPLIFY({}, 1000))", g(nearly_straight)),
        json!(2),
    )
    .await;
    // A tolerance far below the deviation keeps every vertex. The middle vertex
    // sits ~1e-7 degrees (~1 cm) off the chord, so 1 mm must not remove it.
    ctx.eq(
        "ST_SIMPLIFY keeps vertices outside the tolerance",
        &format!("ST_NUMPOINTS(ST_SIMPLIFY({}, 0.001))", g(nearly_straight)),
        json!(3),
    )
    .await;
    // A Point has nothing to drop and must come back unmoved — the projection
    // round trip used to shift it in its last decimals.
    ctx.eq(
        "ST_SIMPLIFY of a Point is exactly the Point",
        "ST_ASGEOJSON(ST_SIMPLIFY(ST_POINT(8.5,47.4), 1000))",
        json!(r#"{"type":"Point","coordinates":[8.5,47.4]}"#),
    )
    .await;
}
