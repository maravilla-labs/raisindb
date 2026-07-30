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

/// REGRESSION: `ST_BUFFER` buffers the actual geometry in **metres**.
///
/// Two independent ways this used to be wrong, and both are pinned here:
///
/// 1. The old implementation collapsed every non-Point to its **centroid** and
///    drew a 32-gon. A long thin shape's buffer is a corridor, not a disc, and
///    their areas differ by orders of magnitude.
/// 2. `geo`'s `Buffer` is planar and works in the geometry's own units, so a
///    bare `.buffer(50)` on EPSG:4326 means 50 **degrees**. The area assertions
///    below are in square metres and a degree-buffer overshoots by ~1e10.
pub(super) async fn buffer(ctx: &mut Ctx) {
    // A Point buffered by 100 m is a disc: pi * 100^2 = 31416 m^2. The result is
    // a polygonal approximation (inscribed), so it is slightly under; 2% covers
    // the segment count without admitting a degree-unit error.
    let disc = std::f64::consts::PI * 100.0 * 100.0;
    ctx.near_rel(
        "ST_BUFFER of a Point is a disc of the right area IN METRES",
        "ST_AREA(ST_BUFFER(ST_POINT(8.5, 47.4), 100))",
        disc,
        0.02,
    )
    .await;
    ctx.eq(
        "ST_BUFFER returns an areal geometry",
        "ST_GEOMETRYTYPE(ST_BUFFER(ST_POINT(8.5, 47.4), 100))",
        json!("ST_Polygon"),
    )
    .await;

    // A LineString buffered by 50 m is a corridor:
    //   2 * 50 * length + pi * 50^2  (a rectangle plus two end caps)
    // The fixture line is 0.09 degrees of latitude at 47.4N ~= 10007.6 m, so
    //   2 * 50 * 10007.6 + pi * 2500 = 1000760 + 7854 = 1008614 m^2.
    // A centroid disc of radius 50 would be 7854 m^2 — 128x smaller. This single
    // assertion is what a centroid implementation cannot survive.
    let line = r#"{"type":"LineString","coordinates":[[8.5,47.4],[8.5,47.49]]}"#;
    ctx.near_rel(
        "REGRESSION ST_BUFFER of a LineString is a corridor, not a centroid disc",
        &format!("ST_AREA(ST_BUFFER({}, 50))", g(line)),
        1_008_614.0,
        0.02,
    )
    .await;
    // Stated as an inequality too, so the intent survives a tolerance edit: the
    // corridor must be at least 50x a centroid disc.
    ctx.num_matches(
        "REGRESSION a LineString buffer is far larger than a centroid disc",
        &format!(
            "ST_AREA(ST_BUFFER({}, 50)) / ST_AREA(ST_BUFFER(ST_CENTROID({}), 50))",
            g(line),
            g(line)
        ),
        "at least 50x the centroid disc",
        |r| r > 50.0,
    )
    .await;

    // A Polygon buffered outward grows by roughly perimeter*d + pi*d^2.
    // The fixture is a 0.001 x 0.001 degree box at 47.4N: 75.36 m x 110.6 m.
    // area = 8335 m^2; buffered by 20 m:
    //   (75.36 + 40) * (110.6 + 40) - 4*400 + pi*400 = 17374 - 1600 + 1257 = 17031
    let boxp = r#"{"type":"Polygon","coordinates":[[[8.5,47.4],[8.501,47.4],[8.501,47.401],[8.5,47.401],[8.5,47.4]]]}"#;
    ctx.num_matches(
        "REGRESSION ST_BUFFER of a Polygon grows the polygon itself",
        &format!("ST_AREA(ST_BUFFER({}, 20))", g(boxp)),
        "between 16000 and 18500 sq m",
        |a| a > 16_000.0 && a < 18_500.0,
    )
    .await;
    // The buffer must strictly contain the original.
    ctx.is_true(
        "ST_BUFFER contains the original geometry",
        &format!("ST_COVERS(ST_BUFFER({0}, 20), {0})", g(boxp)),
    )
    .await;

    // The 3-arg overload controls segments per quarter circle. More segments
    // means a closer approximation to the true disc, hence a LARGER area.
    ctx.is_true(
        "ST_BUFFER(g, d, segments) refines the approximation",
        "ST_AREA(ST_BUFFER(ST_POINT(8.5,47.4), 100, 16)) > ST_AREA(ST_BUFFER(ST_POINT(8.5,47.4), 100, 2))",
    )
    .await;

    // Multi* and GeometryCollection inputs must be accepted.
    for label in ["mpt", "mls", "mpoly", "gc"] {
        ctx.num_matches(
            &format!("ST_BUFFER accepts {label}"),
            &format!("ST_AREA(ST_BUFFER({}, 1000))", expr(label)),
            "a positive area",
            |a| a > 0.0,
        )
        .await;
    }
}
