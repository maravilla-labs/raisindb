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

use super::super::super::fixtures::expr;
use super::super::super::harness::{g, Ctx};
use super::super::SQUARE;

pub(crate) async fn boundary(ctx: &mut Ctx) {
    let sq = g(SQUARE);
    // The corner point (0,0) lies ON the boundary. This is the case that
    // distinguishes COVERS/COVEREDBY from CONTAINS/WITHIN, and the reason both
    // pairs exist.
    ctx.is_false(
        "ST_CONTAINS is FALSE for a boundary Point",
        &format!("ST_CONTAINS({sq}, ST_POINT(0,0))"),
    )
    .await;
    ctx.is_true(
        "ST_COVERS is TRUE for the same boundary Point",
        &format!("ST_COVERS({sq}, ST_POINT(0,0))"),
    )
    .await;
    ctx.is_false(
        "ST_WITHIN is FALSE for a boundary Point",
        &format!("ST_WITHIN(ST_POINT(0,0), {sq})"),
    )
    .await;
    ctx.is_true(
        "ST_COVEREDBY is TRUE for the same boundary Point",
        &format!("ST_COVEREDBY(ST_POINT(0,0), {sq})"),
    )
    .await;
    // A boundary point touches, and intersects.
    ctx.is_true(
        "ST_INTERSECTS is TRUE for a boundary Point",
        &format!("ST_INTERSECTS(ST_POINT(0,0), {sq})"),
    )
    .await;

    // An interior ring is a hole: a point inside the hole is NOT in the polygon.
    let holed = expr("poly_hole"); // 4x4 shell, hole [1,1]..[2,2]
    ctx.is_false(
        "a Point inside an interior ring is not contained",
        &format!("ST_CONTAINS({holed}, ST_POINT(1.5,1.5))"),
    )
    .await;
    ctx.is_true(
        "a Point in the shell but outside the hole is contained",
        &format!("ST_CONTAINS({holed}, ST_POINT(3,3))"),
    )
    .await;
    ctx.is_true(
        "a Point inside an interior ring is disjoint from the polygon",
        &format!("ST_DISJOINT({holed}, ST_POINT(1.5,1.5))"),
    )
    .await;
}

pub(crate) async fn relate(ctx: &mut Ctx) {
    // A Point in a polygon's interior: interior/interior = 0, and the point has
    // no boundary. The exact DE-9IM string is a published JTS/PostGIS value.
    ctx.eq(
        "ST_RELATE Point in Polygon interior",
        &format!("ST_RELATE(ST_POINT(1,1), {})", g(SQUARE)),
        json!("0FFFFF212"),
    )
    .await;
    // Two identical polygons: the canonical topological-equality matrix.
    ctx.eq(
        "ST_RELATE identical polygons is 2FFF1FFF2",
        &format!("ST_RELATE({}, {})", g(SQUARE), g(SQUARE)),
        json!("2FFF1FFF2"),
    )
    .await;
    // Disjoint geometries: everything F except the exterior/exterior cell.
    ctx.eq(
        "ST_RELATE disjoint points is FF0FFF0F2",
        "ST_RELATE(ST_POINT(0,0), ST_POINT(9,9))",
        json!("FF0FFF0F2"),
    )
    .await;

    // The 3-arg pattern form.
    ctx.is_true(
        "ST_RELATE pattern matches 'within'",
        &format!("ST_RELATE(ST_POINT(1,1), {}, 'T*F**F***')", g(SQUARE)),
    )
    .await;
    ctx.is_false(
        "ST_RELATE pattern that does not match",
        &format!("ST_RELATE(ST_POINT(9,9), {}, 'T*F**F***')", g(SQUARE)),
    )
    .await;
    // ST_RELATE must agree with the named predicate it encodes.
    ctx.is_true(
        "ST_RELATE 'T*F**F***' agrees with ST_WITHIN",
        &format!(
            "ST_RELATE(ST_POINT(1,1), {0}, 'T*F**F***') = ST_WITHIN(ST_POINT(1,1), {0})",
            g(SQUARE)
        ),
    )
    .await;
    // An invalid pattern must ERROR, not quietly return false. `matches` parses
    // lazily and would return Ok(false) on a bad character after the first
    // mismatch, so this is validated eagerly.
    ctx.errors_with(
        "ST_RELATE rejects an invalid pattern",
        "ST_RELATE(ST_POINT(1,1), ST_POINT(1,1), 'BADPATTERN')",
        "invalid DE-9IM pattern",
    )
    .await;
    ctx.errors_with(
        "ST_RELATE rejects a wrong-length pattern",
        "ST_RELATE(ST_POINT(1,1), ST_POINT(1,1), 'TTT')",
        "DE-9IM",
    )
    .await;
}
