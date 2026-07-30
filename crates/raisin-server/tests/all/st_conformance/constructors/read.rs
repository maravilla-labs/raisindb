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

pub(super) async fn accessors(ctx: &mut Ctx) {
    ctx.near("ST_X of a Point", "ST_X(ST_POINT(8.5, 47.4))", 8.5, 1e-12)
        .await;
    ctx.near("ST_Y of a Point", "ST_Y(ST_POINT(8.5, 47.4))", 47.4, 1e-12)
        .await;
    // ST_X/ST_Y are NULL, not errors, for a geometry that is not one location.
    ctx.is_null(
        "ST_X of a LineString is NULL",
        &format!("ST_X({})", expr("ls")),
    )
    .await;

    for (label, want) in [
        ("pt", "ST_Point"),
        ("ls", "ST_LineString"),
        ("poly_hole", "ST_Polygon"),
        ("mpt", "ST_MultiPoint"),
        ("mls", "ST_MultiLineString"),
        ("mpoly", "ST_MultiPolygon"),
        ("gc", "ST_GeometryCollection"),
    ] {
        ctx.eq(
            &format!("ST_GEOMETRYTYPE({label})"),
            &format!("ST_GEOMETRYTYPE({})", expr(label)),
            json!(want),
        )
        .await;
    }

    // ST_NUMPOINTS counts every coordinate, interior rings included:
    // shell 5 + hole 5 = 10.
    ctx.eq(
        "ST_NUMPOINTS counts interior ring vertices",
        &format!("ST_NUMPOINTS({})", expr("poly_hole")),
        json!(10),
    )
    .await;
    ctx.eq(
        "ST_NUMPOINTS of a LineString",
        &format!("ST_NUMPOINTS({})", expr("ls")),
        json!(3),
    )
    .await;

    // ST_NUMGEOMETRIES: 1 for a single geometry, N for a Multi*/collection.
    ctx.eq(
        "ST_NUMGEOMETRIES of a Point is 1",
        "ST_NUMGEOMETRIES(ST_POINT(0,0))",
        json!(1),
    )
    .await;
    ctx.eq(
        "ST_NUMGEOMETRIES of a MultiPolygon of 2",
        &format!("ST_NUMGEOMETRIES({})", expr("mpoly")),
        json!(2),
    )
    .await;
    ctx.eq(
        "ST_NUMGEOMETRIES of a 3-member GeometryCollection",
        &format!("ST_NUMGEOMETRIES({})", expr("gc")),
        json!(3),
    )
    .await;
}

pub(super) async fn line_access(ctx: &mut Ctx) {
    let ls = expr("ls"); // [[0,0],[2,0],[2,2]]

    ctx.eq(
        "ST_STARTPOINT",
        &format!("ST_ASGEOJSON(ST_STARTPOINT({ls}))"),
        json!(r#"{"type":"Point","coordinates":[0.0,0.0]}"#),
    )
    .await;
    ctx.eq(
        "ST_ENDPOINT",
        &format!("ST_ASGEOJSON(ST_ENDPOINT({ls}))"),
        json!(r#"{"type":"Point","coordinates":[2.0,2.0]}"#),
    )
    .await;
    // ST_POINTN is 1-based, matching PostGIS.
    ctx.eq(
        "ST_POINTN(ls, 2) is the middle vertex",
        &format!("ST_ASGEOJSON(ST_POINTN({ls}, 2))"),
        json!(r#"{"type":"Point","coordinates":[2.0,0.0]}"#),
    )
    .await;
    ctx.is_null(
        "ST_POINTN past the end is NULL",
        &format!("ST_POINTN({ls}, 99)"),
    )
    .await;
    ctx.is_null(
        "ST_STARTPOINT of a non-linear geometry is NULL",
        "ST_STARTPOINT(ST_POINT(0,0))",
    )
    .await;

    // ST_LINEINTERPOLATEPOINT measures by LENGTH, not by vertex index. The
    // fixture is two equal-length 2-degree legs, so the halfway point is the
    // shared vertex (2,0) — a vertex-index implementation would land elsewhere.
    ctx.near(
        "ST_LINEINTERPOLATEPOINT(0.5) is halfway BY LENGTH",
        &format!("ST_X(ST_LINEINTERPOLATEPOINT({ls}, 0.5))"),
        2.0,
        1e-6,
    )
    .await;
    ctx.near(
        "ST_LINEINTERPOLATEPOINT(0.5) y ordinate",
        &format!("ST_Y(ST_LINEINTERPOLATEPOINT({ls}, 0.5))"),
        0.0,
        1e-6,
    )
    .await;
    ctx.near(
        "ST_LINEINTERPOLATEPOINT(0) is the start",
        &format!("ST_X(ST_LINEINTERPOLATEPOINT({ls}, 0))"),
        0.0,
        1e-9,
    )
    .await;
    ctx.near(
        "ST_LINEINTERPOLATEPOINT(1) is the end",
        &format!("ST_Y(ST_LINEINTERPOLATEPOINT({ls}, 1))"),
        2.0,
        1e-6,
    )
    .await;
}

pub(super) async fn three_d(ctx: &mut Ctx) {
    let p3 = expr("pt3d"); // [1,1,250]

    ctx.eq(
        "ST_Z reads the altitude",
        &format!("ST_Z({p3})"),
        json!(250.0),
    )
    .await;
    ctx.is_null("ST_Z of a 2-D Point is NULL", "ST_Z(ST_POINT(1,1))")
        .await;
    ctx.is_null(
        "ST_Z of a non-Point is NULL",
        &format!("ST_Z({})", expr("ls")),
    )
    .await;

    ctx.eq(
        "ST_NDIMS of a 3-D Point",
        &format!("ST_NDIMS({p3})"),
        json!(3),
    )
    .await;
    ctx.eq(
        "ST_NDIMS of a 2-D Point",
        "ST_NDIMS(ST_POINT(1,1))",
        json!(2),
    )
    .await;

    ctx.eq(
        "ST_ZMIN of a 3-D Point",
        &format!("ST_ZMIN({p3})"),
        json!(250.0),
    )
    .await;
    ctx.eq(
        "ST_ZMAX of a 3-D Point",
        &format!("ST_ZMAX({p3})"),
        json!(250.0),
    )
    .await;
    ctx.is_null(
        "ST_ZMIN of a 2-D geometry is NULL",
        &format!("ST_ZMIN({})", expr("ls")),
    )
    .await;

    // A LineString with two different altitudes exercises the z_range, not just
    // a single ordinate.
    let zline =
        "ST_GEOMFROMGEOJSON('{\"type\":\"LineString\",\"coordinates\":[[0,0,10],[1,1,90]]}')";
    ctx.eq(
        "ST_ZMIN over a varying z_range",
        &format!("ST_ZMIN({zline})"),
        json!(10.0),
    )
    .await;
    ctx.eq(
        "ST_ZMAX over a varying z_range",
        &format!("ST_ZMAX({zline})"),
        json!(90.0),
    )
    .await;

    // ST_FORCE2D edits the GeoJSON in place rather than round-tripping through
    // `geo`, so the surviving ordinates keep the exact JSON number formatting
    // they were written with — here the integers from the fixture literal.
    ctx.eq(
        "ST_FORCE2D drops the altitude",
        &format!("ST_ASGEOJSON(ST_FORCE2D({p3}))"),
        json!(r#"{"type":"Point","coordinates":[1,1]}"#),
    )
    .await;
    ctx.eq(
        "ST_FORCE2D makes ST_NDIMS 2",
        &format!("ST_NDIMS(ST_FORCE2D({p3}))"),
        json!(2),
    )
    .await;
    ctx.eq(
        "ST_FORCE3D adds an altitude",
        "ST_Z(ST_FORCE3D(ST_POINT(1,1), 42))",
        json!(42.0),
    )
    .await;
    // ST_FORCE3D keeps an altitude that is already present.
    ctx.eq(
        "ST_FORCE3D keeps an existing altitude",
        &format!("ST_Z(ST_FORCE3D({p3}, 42))"),
        json!(250.0),
    )
    .await;

    // ST_3DDISTANCE is hypot(geodesic horizontal, vertical gap). Purely vertical:
    // the horizontal term is 0 so the answer is exactly the altitude difference.
    ctx.near(
        "ST_3DDISTANCE purely vertical is the altitude gap",
        "ST_3DDISTANCE(ST_FORCE3D(ST_POINT(0,0), 0), ST_FORCE3D(ST_POINT(0,0), 30))",
        30.0,
        1e-6,
    )
    .await;
    // Horizontal 0.001 deg of latitude = 110.574 m; vertical 100 m.
    // hypot(110.5744, 100) = 148.978 m. Haversine's spherical value for the
    // horizontal leg is 111.195 m, giving hypot = 149.44 — so a 1 m tolerance
    // covers sphere-vs-ellipsoid without admitting a wrong formula.
    ctx.near(
        "ST_3DDISTANCE combines horizontal and vertical",
        "ST_3DDISTANCE(ST_FORCE3D(ST_POINT(0,0), 0), ST_FORCE3D(ST_POINT(0,0.001), 100))",
        149.2,
        1.0,
    )
    .await;
    ctx.is_null(
        "ST_3DDISTANCE with a 2-D operand is NULL",
        "ST_3DDISTANCE(ST_POINT(0,0), ST_FORCE3D(ST_POINT(0,0), 30))",
    )
    .await;

    ctx.is_true(
        "ST_3DDWITHIN true inside the radius",
        "ST_3DDWITHIN(ST_FORCE3D(ST_POINT(0,0), 0), ST_FORCE3D(ST_POINT(0,0), 30), 35)",
    )
    .await;
    ctx.is_false(
        "ST_3DDWITHIN false outside the radius",
        "ST_3DDWITHIN(ST_FORCE3D(ST_POINT(0,0), 0), ST_FORCE3D(ST_POINT(0,0), 30), 25)",
    )
    .await;
    // The vertical gap is what distinguishes 3-D from 2-D here: horizontally the
    // two points are identical, so a 2-D ST_DWITHIN would say true at any radius.
    ctx.is_true(
        "2-D ST_DWITHIN ignores the altitude that ST_3DDWITHIN rejects",
        "ST_DWITHIN(ST_FORCE3D(ST_POINT(0,0), 0), ST_FORCE3D(ST_POINT(0,0), 30), 1)",
    )
    .await;
}
