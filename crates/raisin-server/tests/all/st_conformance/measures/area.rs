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
use super::{DEG_LAT_EQUATOR_M, DEG_LON_EQUATOR_M};

pub(super) async fn area(ctx: &mut Ctx) {
    // A 0.01 deg x 0.01 deg box at the equator. Ellipsoidal area is the product
    // of the two degree lengths scaled by 0.01 each:
    //   (110574.39 * 0.01) * (111319.49 * 0.01) = 1105.7439 * 1113.1949
    //                                            = 1230907.0 m^2
    let small_box =
        r#"{"type":"Polygon","coordinates":[[[0,0],[0.01,0],[0.01,0.01],[0,0.01],[0,0]]]}"#;
    let expected = (DEG_LAT_EQUATOR_M * 0.01) * (DEG_LON_EQUATOR_M * 0.01);
    ctx.near_rel(
        "ST_AREA is ellipsoidal square metres",
        &format!("ST_AREA({})", g(small_box)),
        expected,
        1e-4,
    )
    .await;

    // Areal-only: puntal and linear components contribute 0 rather than erroring.
    ctx.eq(
        "ST_AREA of a Point is 0",
        "ST_AREA(ST_POINT(1,1))",
        json!(0.0),
    )
    .await;
    ctx.eq(
        "ST_AREA of a LineString is 0",
        &format!("ST_AREA({})", expr("ls")),
        json!(0.0),
    )
    .await;

    // MultiPolygon sums its members; GeometryCollection sums its areal ones.
    ctx.num_matches(
        "ST_AREA of a MultiPolygon sums its members",
        &format!("ST_AREA({})", expr("mpoly")),
        "roughly two 1-degree squares (> 2.4e10)",
        |a| a > 2.4e10 && a < 2.5e10,
    )
    .await;
    ctx.num_matches(
        "ST_AREA of a GeometryCollection counts only areal members",
        &format!("ST_AREA({})", expr("gc")),
        "one 1-degree square (~1.23e10)",
        |a| a > 1.2e10 && a < 1.25e10,
    )
    .await;

    area_is_winding_independent(ctx).await;
    area_of_union(ctx).await;
}

/// REGRESSION: `ST_AREA` must not depend on ring winding.
///
/// This suite found `ST_AREA` returning the **surface area of the Earth**
/// (5.1e14 m^2) for a clockwise-wound polygon, and a large *negative* number for
/// a polygon whose hole was wound the same way as its shell. `geo`'s
/// `geodesic_area_unsigned` declares a winding per ring and `geographiclib`
/// returns `earth_area - |A|` for a ring wound against the declaration.
///
/// Clockwise shells are not exotic — OGC shapefiles use them — so every polygon
/// imported from a shapefile was affected. Fixed in `measure::area` by
/// accumulating `|signed geodesic|` per ring.
async fn area_is_winding_independent(ctx: &mut Ctx) {
    let ccw = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#;
    let cw = r#"{"type":"Polygon","coordinates":[[[0,0],[0,1],[1,1],[1,0],[0,0]]]}"#;

    // The truth for a 1x1 degree cell at the equator, from the two degree
    // lengths: 110574.39 * 111319.49 = 1.23091e10 m^2. The exact geodesic value
    // is slightly lower because a degree of latitude grows with latitude, so a
    // 1% relative tolerance separates "right" from "the Earth".
    let truth = DEG_LAT_EQUATOR_M * DEG_LON_EQUATOR_M;

    ctx.near_rel(
        "ST_AREA of a CCW square",
        &format!("ST_AREA({})", g(ccw)),
        truth,
        0.01,
    )
    .await;
    ctx.near_rel(
        "REGRESSION ST_AREA of a CW square is the same, not the Earth",
        &format!("ST_AREA({})", g(cw)),
        truth,
        0.01,
    )
    .await;
    ctx.is_true(
        "REGRESSION ST_AREA is winding-independent",
        &format!("ST_AREA({}) = ST_AREA({})", g(ccw), g(cw)),
    )
    .await;

    // A shell with a hole: 4x4 minus 1x1 degrees. Both windings must agree, and
    // both must equal shell - hole.
    let hole_cw = r#"{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[1,2],[2,2],[2,1],[1,1]]]}"#;
    let hole_ccw = expr("poly_hole"); // hole wound the SAME way as the shell
                                      // The exact ellipsoidal area of the lat/lon rectangle 0..4 x 0..4 is
                                      // 196_789_484_713 m^2 and of the 1..2 x 1..2 hole is 12_304_814_950, so
                                      // shell - hole = 184_484_669_763. A geodesic polygon through the same
                                      // corners runs a shade larger (its edges are geodesics, not parallels of
                                      // latitude), hence the 1e-3 window. Before the winding fix: -5.1e14.
    ctx.near_rel(
        "REGRESSION ST_AREA subtracts a same-wound hole",
        &format!("ST_AREA({hole_ccw})"),
        184_484_669_763.0,
        1e-3,
    )
    .await;
    // There is no ABS() in this SQL dialect, so the ratio is taken in SQL and
    // the absolute value in the assertion closure.
    ctx.num_matches(
        "REGRESSION hole winding does not change ST_AREA",
        &format!(
            "(ST_AREA({hole_ccw}) - ST_AREA({})) / ST_AREA({hole_ccw})",
            g(hole_cw)
        ),
        "a relative difference below 1e-9 either way",
        |d| d.abs() < 1e-9,
    )
    .await;

    // And the hole really is subtracted: shell-with-hole < shell alone.
    let shell_only = r#"{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]]]}"#;
    ctx.is_true(
        "ST_AREA of a shell with a hole is less than the shell",
        &format!("ST_AREA({hole_ccw}) < ST_AREA({})", g(shell_only)),
    )
    .await;
}

/// REGRESSION: `ST_AREA(ST_UNION(a, b))` when the union yields a MultiPolygon.
///
/// The named failure of the old Polygon-only implementation: `ST_UNION` produced
/// a MultiPolygon that no measurement function would accept.
async fn area_of_union(ctx: &mut Ctx) {
    let a = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#;
    let far = r#"{"type":"Polygon","coordinates":[[[5,5],[6,5],[6,6],[5,6],[5,5]]]}"#;

    ctx.eq(
        "ST_UNION of disjoint polygons is a MultiPolygon",
        &format!("ST_GEOMETRYTYPE(ST_UNION({}, {}))", g(a), g(far)),
        json!("ST_MultiPolygon"),
    )
    .await;
    // The union is disjoint, so its area must equal the sum of the parts exactly.
    ctx.num_matches(
        "REGRESSION ST_AREA(ST_UNION(a,b)) over a MultiPolygon",
        &format!(
            "(ST_AREA(ST_UNION({0}, {1})) - (ST_AREA({0}) + ST_AREA({1}))) / ST_AREA({0})",
            g(a),
            g(far)
        ),
        "a relative difference below 1e-9 from the sum of the parts",
        |d| d.abs() < 1e-9,
    )
    .await;
}
