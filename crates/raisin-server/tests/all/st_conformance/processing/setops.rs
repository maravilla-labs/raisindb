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

use super::super::fixtures::expr;
use super::super::harness::{g, Ctx};

pub(super) async fn setops(ctx: &mut Ctx) {
    let a = r#"{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}"#;
    let b = r#"{"type":"Polygon","coordinates":[[[1,1],[3,1],[3,3],[1,3],[1,1]]]}"#;

    // Two 2x2 squares overlapping in a 1x1 corner.
    // union = 4 + 4 - 1 = 7 sq degrees; intersection = 1; difference = 3;
    // symdifference = 6. Asserted as ratios against the intersection so the
    // check is independent of the degree-to-metre factor.
    ctx.near_rel(
        "ST_INTERSECTION area is the 1x1 overlap",
        &format!("ST_AREA(ST_INTERSECTION({}, {}))", g(a), g(b)),
        1.2308e10,
        0.02,
    )
    .await;
    ctx.num_matches(
        "ST_UNION area is 7x the overlap",
        &format!(
            "ST_AREA(ST_UNION({0}, {1})) / ST_AREA(ST_INTERSECTION({0}, {1}))",
            g(a),
            g(b)
        ),
        "about 7",
        |r| (r - 7.0).abs() < 0.05,
    )
    .await;
    ctx.num_matches(
        "ST_DIFFERENCE area is 3x the overlap",
        &format!(
            "ST_AREA(ST_DIFFERENCE({0}, {1})) / ST_AREA(ST_INTERSECTION({0}, {1}))",
            g(a),
            g(b)
        ),
        "about 3",
        |r| (r - 3.0).abs() < 0.05,
    )
    .await;
    ctx.num_matches(
        "ST_SYMDIFFERENCE area is 6x the overlap",
        &format!(
            "ST_AREA(ST_SYMDIFFERENCE({0}, {1})) / ST_AREA(ST_INTERSECTION({0}, {1}))",
            g(a),
            g(b)
        ),
        "about 6",
        |r| (r - 6.0).abs() < 0.05,
    )
    .await;
    // union = intersection + symdifference, exactly.
    ctx.num_matches(
        "ST_UNION = ST_INTERSECTION + ST_SYMDIFFERENCE",
        &format!(
            "(ST_AREA(ST_UNION({0}, {1})) - (ST_AREA(ST_INTERSECTION({0}, {1})) + ST_AREA(ST_SYMDIFFERENCE({0}, {1})))) / ST_AREA(ST_UNION({0}, {1}))",
            g(a),
            g(b)
        ),
        "a relative difference below 1e-9 either way",
        |d| d.abs() < 1e-9,
    )
    .await;

    // Disjoint operands: difference is the whole of A, intersection is empty.
    let far = r#"{"type":"Polygon","coordinates":[[[9,9],[10,9],[10,10],[9,10],[9,9]]]}"#;
    ctx.is_true(
        "ST_INTERSECTION of disjoint polygons is empty",
        &format!("ST_ISEMPTY(ST_INTERSECTION({}, {}))", g(a), g(far)),
    )
    .await;
    ctx.is_true(
        "ST_DIFFERENCE with a disjoint operand is unchanged",
        &format!("ST_EQUALS(ST_DIFFERENCE({0}, {1}), {0})", g(a), g(far)),
    )
    .await;
    // Self-difference is empty; self-union is the geometry.
    ctx.is_true(
        "ST_DIFFERENCE of a polygon with itself is empty",
        &format!("ST_ISEMPTY(ST_DIFFERENCE({0}, {0}))", g(a)),
    )
    .await;
    ctx.is_true(
        "ST_UNION of a polygon with itself equals it",
        &format!("ST_EQUALS(ST_UNION({0}, {0}), {0})", g(a)),
    )
    .await;

    // Non-areal operands: `geo`'s BooleanOps is Polygon/MultiPolygon only, so
    // these run through bespoke segment algebra and are the likeliest to be
    // silently wrong. Two collinear overlapping segments: union covers 0..3,
    // intersection is the shared 1..2 stretch.
    let l1 = r#"{"type":"LineString","coordinates":[[0,0],[2,0]]}"#;
    let l2 = r#"{"type":"LineString","coordinates":[[1,0],[3,0]]}"#;
    ctx.num_matches(
        "ST_UNION of collinear LineStrings spans the whole extent",
        &format!("ST_LENGTH(ST_UNION({}, {}))", g(l1), g(l2)),
        "about 3 degrees of length",
        |m| (m / 111_195.08 - 3.0).abs() < 0.02,
    )
    .await;
    ctx.num_matches(
        "ST_INTERSECTION of collinear LineStrings is the shared stretch",
        &format!("ST_LENGTH(ST_INTERSECTION({}, {}))", g(l1), g(l2)),
        "about 1 degree of length",
        |m| (m / 111_195.08 - 1.0).abs() < 0.02,
    )
    .await;
    // A CROSSING (not collinear) removes no length from a 1-D difference — the
    // subtle rule that distinguishes a real implementation from a plausible one.
    let cross = r#"{"type":"LineString","coordinates":[[1,-1],[1,1]]}"#;
    ctx.num_matches(
        "ST_DIFFERENCE unchanged by a merely crossing line",
        &format!(
            "ST_LENGTH(ST_DIFFERENCE({0}, {1})) / ST_LENGTH({0})",
            g(l1),
            g(cross)
        ),
        "essentially 1 (no length removed)",
        |r| (r - 1.0).abs() < 1e-6,
    )
    .await;
    // ...whereas a collinear overlap DOES remove length.
    ctx.num_matches(
        "ST_DIFFERENCE removes a collinear overlap",
        &format!(
            "ST_LENGTH(ST_DIFFERENCE({0}, {1})) / ST_LENGTH({0})",
            g(l1),
            g(l2)
        ),
        "about half the length remaining",
        |r| (r - 0.5).abs() < 0.02,
    )
    .await;

    // Mixed dimensions and collections must not error.
    for (name, x, y) in [
        ("Point/Polygon", "ST_POINT(1,1)".to_string(), g(a)),
        ("LineString/Polygon", expr("ls"), g(a)),
        ("MultiPolygon/Polygon", expr("mpoly"), g(a)),
        ("GeometryCollection/Polygon", expr("gc"), g(a)),
    ] {
        for op in [
            "ST_UNION",
            "ST_INTERSECTION",
            "ST_DIFFERENCE",
            "ST_SYMDIFFERENCE",
        ] {
            let sql = format!("SELECT ST_ISEMPTY({op}({x}, {y})) AS r");
            ctx.cov.record_sql(&sql, "set-op type matrix");
            if let Err(e) = ctx.sql(&sql).await {
                println!("  [FAIL] {op} {name}: {e}");
                ctx.failures.push(format!("{op} {name}: {e}"));
            }
        }
    }
    println!("  [ ok ] set operations accept mixed-dimension and collection operands");
}
