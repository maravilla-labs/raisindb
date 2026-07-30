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

use super::super::super::harness::{g, Ctx};
use super::super::SQUARE;

pub(crate) async fn concrete(ctx: &mut Ctx) {
    let sq = g(SQUARE);

    ctx.is_true(
        "ST_INTERSECTS Point inside Polygon",
        &format!("ST_INTERSECTS(ST_POINT(1,1), {sq})"),
    )
    .await;
    ctx.is_false(
        "ST_INTERSECTS Point outside Polygon",
        &format!("ST_INTERSECTS(ST_POINT(9,9), {sq})"),
    )
    .await;
    ctx.is_true(
        "ST_CONTAINS Polygon contains interior Point",
        &format!("ST_CONTAINS({sq}, ST_POINT(1,1))"),
    )
    .await;
    ctx.is_true(
        "ST_WITHIN Point within Polygon",
        &format!("ST_WITHIN(ST_POINT(1,1), {sq})"),
    )
    .await;
    ctx.is_true(
        "ST_DISJOINT for a far-away Point",
        &format!("ST_DISJOINT(ST_POINT(9,9), {sq})"),
    )
    .await;

    // A polygon strictly inside another: CONTAINS true, OVERLAPS false.
    let inner = r#"{"type":"Polygon","coordinates":[[[0.5,0.5],[1,0.5],[1,1],[0.5,1],[0.5,0.5]]]}"#;
    ctx.is_true(
        "ST_CONTAINS Polygon contains inner Polygon",
        &format!("ST_CONTAINS({sq}, {})", g(inner)),
    )
    .await;
    ctx.is_false(
        "ST_OVERLAPS is false for containment",
        &format!("ST_OVERLAPS({sq}, {})", g(inner)),
    )
    .await;

    // ST_CONTAINS(polygon, linestring) — one of the commonest real queries, and
    // one the old per-pair implementation raised "not supported" for.
    ctx.is_true(
        "ST_CONTAINS Polygon contains interior LineString",
        &format!(
            "ST_CONTAINS({sq}, {})",
            g(r#"{"type":"LineString","coordinates":[[0.5,0.5],[1.5,1.5]]}"#)
        ),
    )
    .await;
    // The square's own diagonal IS contained: endpoints on the boundary are not
    // in the exterior, and the interiors intersect.
    ctx.is_true(
        "ST_CONTAINS Polygon contains its own diagonal",
        &format!(
            "ST_CONTAINS({sq}, {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,2]]}"#)
        ),
    )
    .await;
}

/// The specific cases the audit named as broken.
pub(crate) async fn regressions(ctx: &mut Ctx) {
    let sq = g(SQUARE);

    // ST_INTERSECTS Point/LineString and LineString/LineString.
    ctx.is_true(
        "REGRESSION ST_INTERSECTS Point on LineString",
        &format!(
            "ST_INTERSECTS(ST_POINT(1,1), {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,2]]}"#)
        ),
    )
    .await;
    ctx.is_true(
        "REGRESSION ST_INTERSECTS crossing LineStrings",
        &format!(
            "ST_INTERSECTS({}, {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,2]]}"#),
            g(r#"{"type":"LineString","coordinates":[[0,2],[2,0]]}"#)
        ),
    )
    .await;

    // ST_CROSSES with a Point argument: hardcoded false before.
    // A MultiPoint with one member inside and one outside a polygon CROSSES it:
    // the interiors meet and the MultiPoint is not contained.
    ctx.is_true(
        "REGRESSION ST_CROSSES MultiPoint/Polygon (was hardcoded false)",
        &format!(
            "ST_CROSSES({}, {})",
            g(r#"{"type":"MultiPoint","coordinates":[[1,1],[9,9]]}"#),
            sq
        ),
    )
    .await;
    // Two LineStrings that genuinely cross.
    ctx.is_true(
        "REGRESSION ST_CROSSES crossing LineStrings",
        &format!(
            "ST_CROSSES({}, {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,2]]}"#),
            g(r#"{"type":"LineString","coordinates":[[0,2],[2,0]]}"#)
        ),
    )
    .await;

    // ST_TOUCHES with a Point argument: hardcoded false before.
    ctx.is_true(
        "REGRESSION ST_TOUCHES Point on a LineString endpoint",
        &format!(
            "ST_TOUCHES(ST_POINT(0,0), {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,1]]}"#)
        ),
    )
    .await;
    ctx.is_true(
        "REGRESSION ST_TOUCHES Point on a Polygon corner",
        &format!("ST_TOUCHES(ST_POINT(0,0), {sq})"),
    )
    .await;
    // A point in the interior does NOT touch — touching needs a boundary meeting.
    ctx.is_false(
        "ST_TOUCHES is false for an interior Point",
        &format!("ST_TOUCHES(ST_POINT(1,1), {sq})"),
    )
    .await;

    // ST_OVERLAPS on genuinely overlapping polygons: the catch-all false case.
    let shifted = r#"{"type":"Polygon","coordinates":[[[1,1],[3,1],[3,3],[1,3],[1,1]]]}"#;
    ctx.is_true(
        "REGRESSION ST_OVERLAPS genuinely overlapping polygons",
        &format!("ST_OVERLAPS({sq}, {})", g(shifted)),
    )
    .await;
    // Partially overlapping LineStrings overlap (same dimension, partial share).
    ctx.is_true(
        "REGRESSION ST_OVERLAPS partially overlapping LineStrings",
        &format!(
            "ST_OVERLAPS({}, {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,0]]}"#),
            g(r#"{"type":"LineString","coordinates":[[1,0],[3,0]]}"#)
        ),
    )
    .await;

    // ST_EQUALS is now exact topological equality: a redundant collinear vertex
    // does not change the geometry, and it LOST the old 1e-8 degree tolerance.
    ctx.is_true(
        "ST_EQUALS ignores a redundant collinear vertex",
        &format!(
            "ST_EQUALS({}, {})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,1],[2,2]]}"#),
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,2]]}"#)
        ),
    )
    .await;
    ctx.is_true(
        "ST_EQUALS a one-member MultiPoint equals the Point",
        &format!(
            "ST_EQUALS({}, ST_POINT(3,4))",
            g(r#"{"type":"MultiPoint","coordinates":[[3,4]]}"#)
        ),
    )
    .await;
    ctx.is_true(
        "ST_EQUALS ignores ring winding",
        &format!(
            "ST_EQUALS({}, {})",
            g(SQUARE),
            g(r#"{"type":"Polygon","coordinates":[[[0,0],[0,2],[2,2],[2,0],[0,0]]]}"#)
        ),
    )
    .await;
    // BREAKING (documented): no coordinate tolerance any more. Two points a
    // millimetre apart are NOT equal; use ST_DWITHIN for fuzzy comparison.
    ctx.is_false(
        "ST_EQUALS has no coordinate tolerance",
        "ST_EQUALS(ST_POINT(1,1), ST_POINT(1.00000001,1))",
    )
    .await;
}
