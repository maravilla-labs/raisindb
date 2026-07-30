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
use super::{DEG_LON_EQUATOR_M, DEG_SPHERE_M};

pub(super) async fn length(ctx: &mut Ctx) {
    // One degree of meridian, measured spherically (Haversine): 111195.08 m.
    ctx.near_rel(
        "ST_LENGTH is spherical metres",
        &format!(
            "ST_LENGTH({})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[0,1]]}"#)
        ),
        DEG_SPHERE_M,
        1e-4,
    )
    .await;

    // The fixture is two 2-degree legs: 2 along the equator, then 2 north.
    ctx.near_rel(
        "ST_LENGTH sums the segments",
        &format!("ST_LENGTH({})", expr("ls")),
        4.0 * DEG_SPHERE_M,
        1e-3,
    )
    .await;

    // MultiLineString sums its components: two 1-degree segments.
    ctx.near_rel(
        "ST_LENGTH of a MultiLineString",
        &format!("ST_LENGTH({})", expr("mls")),
        2.0 * DEG_SPHERE_M,
        1e-3,
    )
    .await;

    // BREAKING (documented): ST_LENGTH of an areal geometry is 0, matching
    // PostGIS. ST_PERIMETER is the function for a polygon's boundary.
    ctx.eq(
        "ST_LENGTH of a Polygon is 0 (use ST_PERIMETER)",
        &format!("ST_LENGTH({})", expr("poly_hole")),
        json!(0.0),
    )
    .await;
    ctx.eq(
        "ST_LENGTH of a Point is 0",
        "ST_LENGTH(ST_POINT(1,1))",
        json!(0.0),
    )
    .await;

    // ST_PERIMETER counts interior rings too: 4x4 shell (16 deg) + 1x1 hole
    // (4 deg) = 20 degrees of boundary.
    ctx.near_rel(
        "ST_PERIMETER includes interior rings",
        &format!("ST_PERIMETER({})", expr("poly_hole")),
        20.0 * DEG_SPHERE_M,
        5e-3,
    )
    .await;
    ctx.eq(
        "ST_PERIMETER of a LineString is 0",
        &format!("ST_PERIMETER({})", expr("ls")),
        json!(0.0),
    )
    .await;
}

pub(super) async fn distance(ctx: &mut Ctx) {
    // Zurich (8.5417, 47.3769) to Vienna (16.3738, 48.2082). The great-circle
    // distance on the GRS80 mean sphere is 592.0 km; published road/air figures
    // for the city pair are ~590-600 km. A 2 km tolerance pins the formula
    // without pinning the ellipsoid model.
    ctx.near(
        "ST_DISTANCE Zurich to Vienna",
        "ST_DISTANCE(ST_POINT(8.5417,47.3769), ST_POINT(16.3738,48.2082))",
        592_066.0,
        2_000.0,
    )
    .await;

    // One degree of longitude at the equator.
    ctx.near_rel(
        "ST_DISTANCE one degree of longitude at the equator",
        "ST_DISTANCE(ST_POINT(0,0), ST_POINT(1,0))",
        DEG_SPHERE_M,
        1e-3,
    )
    .await;

    ctx.eq(
        "ST_DISTANCE from a point to itself is 0",
        "ST_DISTANCE(ST_POINT(1,1), ST_POINT(1,1))",
        json!(0.0),
    )
    .await;

    polygon_distance_is_minimum(ctx).await;

    // ST_DWITHIN. 0.001 deg of latitude = 110.57 m, so 120 m contains it and
    // 100 m does not — a pair that a spherical-vs-ellipsoidal mix-up survives
    // but a unit error does not.
    ctx.is_true(
        "ST_DWITHIN true inside the radius",
        "ST_DWITHIN(ST_POINT(0,0), ST_POINT(0,0.001), 120)",
    )
    .await;
    ctx.is_false(
        "ST_DWITHIN false outside the radius",
        "ST_DWITHIN(ST_POINT(0,0), ST_POINT(0,0.001), 100)",
    )
    .await;
    // ST_DWITHIN must agree with ST_DISTANCE — they are the same question.
    ctx.is_true(
        "ST_DWITHIN agrees with ST_DISTANCE",
        &format!(
            "ST_DWITHIN({0}, {1}, 200000) = (ST_DISTANCE({0}, {1}) <= 200000)",
            expr("pt"),
            "ST_POINT(2,2)"
        ),
    )
    .await;
    // Non-point operands must work, not error.
    ctx.is_true(
        "ST_DWITHIN accepts a Polygon operand",
        &format!("ST_DWITHIN({}, ST_POINT(2,2), 1000000)", expr("poly_hole")),
    )
    .await;
}

/// REGRESSION: `ST_DISTANCE` between two polygons is the true minimum
/// separation, not centroid-to-centroid.
///
/// The shapes are chosen so the two answers differ by orders of magnitude: two
/// long thin bars, 0.01 degrees apart at their nearest edges but 1.5 degrees
/// apart at their centroids. Minimum separation ~1113 m; centroid separation
/// ~167 km. A centroid fallback cannot pass this.
async fn polygon_distance_is_minimum(ctx: &mut Ctx) {
    let left = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#;
    let right = r#"{"type":"Polygon","coordinates":[[[1.01,0],[2,0],[2,1],[1.01,1],[1.01,0]]]}"#;

    // 0.01 degrees of longitude at the equator = 1113.19 m. The implementation
    // projects both operands into one shared UTM zone and measures with
    // Euclidean, so a sub-percent projection error is expected and allowed.
    ctx.near_rel(
        "REGRESSION ST_DISTANCE Polygon/Polygon is minimum separation",
        &format!("ST_DISTANCE({}, {})", g(left), g(right)),
        DEG_LON_EQUATOR_M * 0.01,
        0.01,
    )
    .await;
    // Explicitly: nowhere near the centroid-to-centroid answer.
    ctx.num_matches(
        "REGRESSION ST_DISTANCE Polygon/Polygon is NOT centroid-to-centroid",
        &format!("ST_DISTANCE({}, {})", g(left), g(right)),
        "far below the ~167 km centroid separation",
        |d| d < 10_000.0,
    )
    .await;

    // Overlapping polygons are 0 apart. Centroid-to-centroid would report a
    // large positive number here.
    let overlapping =
        r#"{"type":"Polygon","coordinates":[[[0.5,0.5],[2,0.5],[2,2],[0.5,2],[0.5,0.5]]]}"#;
    ctx.eq(
        "REGRESSION overlapping polygons are 0 apart",
        &format!("ST_DISTANCE({}, {})", g(left), g(overlapping)),
        json!(0.0),
    )
    .await;

    // Point to LineString: the perpendicular distance to the nearest point on
    // the line, not to one of its vertices. The line runs from (1,-5) to (1,5);
    // the nearest point to (0,0) is (1,0), one degree away, while the nearest
    // VERTEX is over 5 degrees away.
    ctx.num_matches(
        "REGRESSION ST_DISTANCE Point/LineString uses the nearest point, not a vertex",
        &format!(
            "ST_DISTANCE(ST_POINT(0,0), {})",
            g(r#"{"type":"LineString","coordinates":[[1,-5],[1,5]]}"#)
        ),
        "about one degree (not five)",
        |d| d > 105_000.0 && d < 118_000.0,
    )
    .await;

    // Multi* and GeometryCollection operands must be accepted.
    ctx.num_matches(
        "ST_DISTANCE accepts MultiPolygon",
        &format!("ST_DISTANCE({}, ST_POINT(0,0))", expr("mpoly")),
        "a finite non-negative distance",
        |d| d.is_finite() && d >= 0.0,
    )
    .await;
    ctx.num_matches(
        "ST_DISTANCE accepts GeometryCollection",
        &format!("ST_DISTANCE({}, ST_POINT(9,9))", expr("gc")),
        "a finite positive distance",
        |d| d.is_finite() && d > 0.0,
    )
    .await;
}

pub(super) async fn azimuth(ctx: &mut Ctx) {
    // North-clockwise radians: due north is 0, due east is pi/2.
    ctx.near(
        "ST_AZIMUTH due north is 0",
        "ST_AZIMUTH(ST_POINT(0,0), ST_POINT(0,1))",
        0.0,
        1e-9,
    )
    .await;
    ctx.near(
        "ST_AZIMUTH due east is pi/2",
        "ST_AZIMUTH(ST_POINT(0,0), ST_POINT(1,0))",
        std::f64::consts::FRAC_PI_2,
        1e-6,
    )
    .await;
    ctx.near(
        "ST_AZIMUTH due south is pi",
        "ST_AZIMUTH(ST_POINT(0,1), ST_POINT(0,0))",
        std::f64::consts::PI,
        1e-6,
    )
    .await;
    ctx.near(
        "ST_AZIMUTH due west is 3pi/2",
        "ST_AZIMUTH(ST_POINT(1,0), ST_POINT(0,0))",
        3.0 * std::f64::consts::FRAC_PI_2,
        1e-6,
    )
    .await;
    // The azimuth from a point to itself is undefined; PostGIS returns NULL.
    ctx.is_null(
        "ST_AZIMUTH from a point to itself is NULL",
        "ST_AZIMUTH(ST_POINT(1,1), ST_POINT(1,1))",
    )
    .await;
}
