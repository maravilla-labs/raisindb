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

pub(super) async fn validation(ctx: &mut Ctx) {
    // REGRESSION: a self-intersecting bow-tie is INVALID. The old array-shape
    // check passed it.
    let bowtie = r#"{"type":"Polygon","coordinates":[[[0,0],[1,1],[1,0],[0,1],[0,0]]]}"#;
    ctx.is_false(
        "REGRESSION ST_ISVALID is false for a self-intersecting bowtie",
        &format!("ST_ISVALID({})", g(bowtie)),
    )
    .await;
    ctx.is_true(
        "ST_ISVALID is true for a simple square",
        &format!(
            "ST_ISVALID({})",
            g(r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#)
        ),
    )
    .await;
    ctx.is_true(
        "ST_ISVALID accepts a polygon with an interior ring",
        &format!("ST_ISVALID({})", expr("poly_hole")),
    )
    .await;

    // ST_ISVALIDREASON explains; the literal "Valid Geometry" for valid input.
    ctx.eq(
        "ST_ISVALIDREASON of valid input",
        &format!(
            "ST_ISVALIDREASON({})",
            g(r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#)
        ),
        json!("Valid Geometry"),
    )
    .await;
    // Assert the text actually mentions the defect rather than merely existing.
    ctx.is_true(
        "ST_ISVALIDREASON mentions self-intersection",
        &format!("ST_ISVALIDREASON({}) LIKE '%self-intersection%'", g(bowtie)),
    )
    .await;

    // ST_MAKEVALID leaves valid input byte-identical, so it is safe over a
    // whole column.
    let square = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}"#;
    // Topological identity, not string identity: ST_MAKEVALID round-trips through
    // `geo` and re-serializes, so an input written with integer ordinates comes
    // back with `1.0` where it went in as `1`. The value is unchanged; only the
    // JSON number formatting is normalized.
    ctx.is_true(
        "ST_MAKEVALID leaves valid input unchanged",
        &format!("ST_EQUALS(ST_MAKEVALID({0}), {0})", g(square)),
    )
    .await;
    ctx.is_true(
        "ST_MAKEVALID repairs a bowtie into a valid geometry",
        &format!("ST_ISVALID(ST_MAKEVALID({}))", g(bowtie)),
    )
    .await;

    // REGRESSION: ST_ISSIMPLE is not a constant true.
    ctx.is_false(
        "REGRESSION ST_ISSIMPLE is false for a self-intersecting LineString",
        &format!(
            "ST_ISSIMPLE({})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[2,2],[2,0],[0,2]]}"#)
        ),
    )
    .await;
    ctx.is_true(
        "ST_ISSIMPLE is true for a simple LineString",
        &format!(
            "ST_ISSIMPLE({})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,0],[2,0]]}"#)
        ),
    )
    .await;
    // A repeated vertex is a degenerate segment, not a self-tangency — the
    // zero-length-segment indexing bug produced a false negative here.
    ctx.is_true(
        "ST_ISSIMPLE tolerates a repeated vertex",
        &format!(
            "ST_ISSIMPLE({})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,0],[1,0],[2,0]]}"#)
        ),
    )
    .await;
    // A closed ring is simple: first == last is not a self-intersection.
    ctx.is_true(
        "ST_ISSIMPLE is true for a closed ring",
        &format!(
            "ST_ISSIMPLE({})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,0],[1,1],[0,0]]}"#)
        ),
    )
    .await;
    // A MultiPoint with a duplicate is not simple.
    ctx.is_false(
        "ST_ISSIMPLE is false for a MultiPoint with duplicates",
        &format!(
            "ST_ISSIMPLE({})",
            g(r#"{"type":"MultiPoint","coordinates":[[1,1],[1,1]]}"#)
        ),
    )
    .await;

    // ST_ISEMPTY.
    ctx.is_true(
        "ST_ISEMPTY of an empty GeometryCollection",
        r#"ST_ISEMPTY(ST_GEOMFROMGEOJSON('{"type":"GeometryCollection","geometries":[]}'))"#,
    )
    .await;
    ctx.is_false(
        "ST_ISEMPTY of a Point is false",
        "ST_ISEMPTY(ST_POINT(1,1))",
    )
    .await;

    // ST_ISCLOSED.
    ctx.is_true(
        "ST_ISCLOSED of a closed LineString",
        &format!(
            "ST_ISCLOSED({})",
            g(r#"{"type":"LineString","coordinates":[[0,0],[1,0],[1,1],[0,0]]}"#)
        ),
    )
    .await;
    ctx.is_false(
        "ST_ISCLOSED of an open LineString",
        &format!("ST_ISCLOSED({})", expr("ls")),
    )
    .await;

    // Every validation function must accept every geometry type.
    for func in ["ST_ISVALID", "ST_ISSIMPLE", "ST_ISEMPTY", "ST_ISCLOSED"] {
        for label in ["pt", "ls", "poly_hole", "mpt", "mls", "mpoly", "gc"] {
            let sql = format!("SELECT {func}({}) AS r", expr(label));
            ctx.cov.record_sql(&sql, "validation type matrix");
            match ctx.sql(&sql).await {
                Ok(rows) => {
                    let v = rows
                        .first()
                        .and_then(|r| r.get("r").cloned())
                        .unwrap_or(serde_json::Value::Null);
                    if !v.is_boolean() {
                        ctx.failures
                            .push(format!("{func}({label}) returned {v}, not a boolean"));
                        println!("  [FAIL] {func}({label}) returned {v}");
                    }
                }
                Err(e) => {
                    ctx.failures.push(format!("{func}({label}): {e}"));
                    println!("  [FAIL] {func}({label}): {e}");
                }
            }
        }
    }
    println!("  [ ok ] validation functions accept every geometry type");
}
