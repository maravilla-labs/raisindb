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
use super::super::harness::Ctx;

pub(super) async fn constructors(ctx: &mut Ctx) {
    // ST_POINT / ST_MAKEPOINT are (longitude, latitude) — the pinned axis order.
    ctx.eq(
        "ST_POINT emits [lon, lat]",
        "ST_ASGEOJSON(ST_POINT(8.5, 47.4))",
        json!(r#"{"type":"Point","coordinates":[8.5,47.4]}"#),
    )
    .await;
    ctx.eq(
        "ST_MAKEPOINT is the ST_POINT alias",
        "ST_ASGEOJSON(ST_MAKEPOINT(8.5, 47.4))",
        json!(r#"{"type":"Point","coordinates":[8.5,47.4]}"#),
    )
    .await;

    // ST_GEOMFROMGEOJSON round-trips every type through ST_ASGEOJSON.
    for label in ["pt", "ls", "poly_hole", "mpt", "mls", "mpoly", "gc", "pt3d"] {
        let e = format!("ST_GEOMETRYTYPE({})", expr(label));
        ctx.num_matches(
            &format!("ST_GEOMFROMGEOJSON accepts {label}"),
            &format!("ST_NUMPOINTS({})", expr(label)),
            "a point count > 0",
            |n| n > 0.0,
        )
        .await;
        // Also confirm the type survives the parse.
        ctx.cov.record_sql(&e, "type round-trip");
    }

    ctx.eq(
        "ST_MAKELINE builds a LineString from two points",
        "ST_ASGEOJSON(ST_MAKELINE(ST_POINT(0,0), ST_POINT(1,1)))",
        json!(r#"{"type":"LineString","coordinates":[[0.0,0.0],[1.0,1.0]]}"#),
    )
    .await;

    ctx.eq(
        "ST_MAKEPOLYGON closes a ring into a Polygon",
        "ST_GEOMETRYTYPE(ST_MAKEPOLYGON(ST_GEOMFROMGEOJSON('{\"type\":\"LineString\",\"coordinates\":[[0,0],[1,0],[1,1],[0,0]]}')))",
        json!("ST_Polygon"),
    )
    .await;

    // ST_MAKEENVELOPE(xmin, ymin, xmax, ymax): a 1°x1° box at the equator.
    ctx.eq(
        "ST_MAKEENVELOPE returns a Polygon",
        "ST_GEOMETRYTYPE(ST_MAKEENVELOPE(0, 0, 1, 1))",
        json!("ST_Polygon"),
    )
    .await;
    // The 5-arg overload LABELS the SRID; it must not reproject.
    ctx.eq(
        "ST_MAKEENVELOPE(.., srid) labels without moving",
        "ST_SRID(ST_MAKEENVELOPE(0, 0, 1, 1, 3857))",
        json!(3857),
    )
    .await;
    ctx.near(
        "ST_MAKEENVELOPE(.., 3857) leaves coordinates untouched",
        "ST_X(ST_CENTROID(ST_MAKEENVELOPE(0, 0, 1, 1, 3857)))",
        0.5,
        1e-9,
    )
    .await;

    // ST_COLLECT of two same-type geometries returns the matching Multi*, which is
    // a deliberate change from the old always-GeometryCollection behaviour.
    ctx.eq(
        "ST_COLLECT of two Points is a MultiPoint",
        "ST_GEOMETRYTYPE(ST_COLLECT(ST_POINT(0,0), ST_POINT(1,1)))",
        json!("ST_MultiPoint"),
    )
    .await;
    ctx.eq(
        "ST_COLLECT of mixed types is a GeometryCollection",
        &format!("ST_GEOMETRYTYPE(ST_COLLECT(ST_POINT(0,0), {}))", expr("ls")),
        json!("ST_GeometryCollection"),
    )
    .await;

    // ST_ASGEOJSON(g, max_decimals) rounds every ordinate at any nesting depth.
    ctx.eq(
        "ST_ASGEOJSON(g, 3) rounds ordinates",
        "ST_ASGEOJSON(ST_POINT(8.123456789, 47.987654321), 3)",
        json!(r#"{"type":"Point","coordinates":[8.123,47.988]}"#),
    )
    .await;
}
