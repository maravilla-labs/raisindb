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

use serde_json::Value;

use super::super::fixtures::geojson;
use super::super::harness::{Ctx, NODE_TYPE, WORKSPACE};
use super::{row_sql, G};

/// REGRESSION: several ST_* calls over the SAME stored property in ONE SELECT.
///
/// This returned all-NULLs before. CSE lifted the repeated
/// `properties->>'g'` into an intermediate projection aliased `__cse_0`, and
/// projection pruning then appended a self-referential pass-through under the
/// SAME alias; the batch projection executor keys output columns by alias, so
/// the second, all-NULL column replaced the real one.
///
/// `SELECT ST_X(g), ST_Y(g)` is about as ordinary as a spatial query gets, so
/// this is the single most user-visible fix in this pass.
pub(super) async fn multi_projection(ctx: &mut Ctx) {
    let sql = format!(
        "SELECT ST_X({G}) AS x, ST_Y({G}) AS y, ST_SRID({G}) AS srid, \
                ST_GEOMETRYTYPE({G}) AS gtype \
         FROM '{WORKSPACE}' \
         WHERE node_type = '{NODE_TYPE}' AND properties->>'label'::String = 'pt'"
    );
    ctx.cov
        .record_sql(&sql, "REGRESSION multiple ST_* in one projection");
    match ctx.sql(&sql).await {
        Ok(rows) => {
            match rows.first() {
                Some(r) => {
                    let x = r.get("x").and_then(|v| v.as_f64());
                    let y = r.get("y").and_then(|v| v.as_f64());
                    let srid = r.get("srid").and_then(|v| v.as_i64());
                    let gtype = r.get("gtype").and_then(|v| v.as_str());
                    let ok = x == Some(1.0)
                        && y == Some(1.0)
                        && srid == Some(4326)
                        && gtype == Some("ST_Point");
                    if ok {
                        println!("  [ ok ] REGRESSION four ST_* calls over one stored property all resolved");
                    } else {
                        println!(
                        "  [FAIL] REGRESSION multi-projection: x={x:?} y={y:?} srid={srid:?} gtype={gtype:?}"
                    );
                        ctx.failures.push(format!(
                        "multi-projection returned x={x:?} y={y:?} srid={srid:?} gtype={gtype:?}"
                    ));
                    }
                }
                None => ctx
                    .failures
                    .push("multi-projection returned no rows".to_string()),
            }
        }
        Err(e) => {
            println!("  [FAIL] multi-projection: {e}");
            ctx.failures.push(format!("multi-projection: {e}"));
        }
    }

    // The same shape with two DIFFERENT functions over the same JSON key, which
    // is the general (non-spatial) form of the same bug.
    let sql2 = format!(
        "SELECT UPPER(properties->>'kind'::String) AS a, LOWER(properties->>'kind'::String) AS b \
         FROM '{WORKSPACE}' \
         WHERE node_type = '{NODE_TYPE}' AND properties->>'label'::String = 'pt'"
    );
    match ctx.sql(&sql2).await {
        Ok(rows) => {
            let a = rows
                .first()
                .and_then(|r| r.get("a"))
                .and_then(|v| v.as_str());
            let b = rows
                .first()
                .and_then(|r| r.get("b"))
                .and_then(|v| v.as_str());
            if a == Some("POINT") && b == Some("point") {
                println!("  [ ok ] REGRESSION two functions over one JSON key both resolved");
            } else {
                println!("  [FAIL] repeated-subexpression projection: a={a:?} b={b:?}");
                ctx.failures.push(format!(
                    "repeated-subexpression projection: a={a:?} b={b:?}"
                ));
            }
        }
        Err(e) => ctx
            .failures
            .push(format!("repeated-subexpression projection: {e}")),
    }
}

/// Spatial predicates used as row filters, which is the real query shape.
pub(super) async fn where_clause(ctx: &mut Ctx) {
    // A small box straddling (1,1). Working out the expected set by hand is the
    // point of the exercise, so here it is in full:
    //   pt (1,1)                      -> IN
    //   pt3d (1,1,250)                -> IN  (z is ignored by 2-D predicates)
    //   gc (contains a Point at 1,1)  -> IN
    //   poly_hole  shell 0..4, hole 1..2 -> IN: the box spans 0.9..1.1, so the
    //              slice below x=1 or y=1 is shell-and-not-hole
    //   mpoly      members 0..1 and 5..6 -> IN: the first member's corner reaches
    //              (1,1), overlapping the box's 0.9..1.0 quadrant
    //   ls  runs along y=0 then x=2    -> OUT
    //   mls at y=0 and y=2             -> OUT
    //   mpt (0,0) (3,1) (1,4)          -> OUT
    //   zurich_*                       -> OUT
    let sql = format!(
        "SELECT properties->>'label'::String AS label FROM '{WORKSPACE}' \
         WHERE node_type = '{NODE_TYPE}' \
           AND ST_INTERSECTS({G}, ST_GEOMFROMGEOJSON('{{\"type\":\"Polygon\",\"coordinates\":[[[0.9,0.9],[1.1,0.9],[1.1,1.1],[0.9,1.1],[0.9,0.9]]]}}')) \
         ORDER BY label"
    );
    ctx.cov
        .record_sql(&sql, "spatial predicate as a row filter");
    match ctx.sql(&sql).await {
        Ok(rows) => {
            let mut labels: Vec<&str> = rows
                .iter()
                .filter_map(|r| r.get("label").and_then(|v| v.as_str()))
                .collect();
            labels.sort_unstable();
            let want = ["gc", "mpoly", "poly_hole", "pt", "pt3d"];
            if labels == want {
                println!(
                    "  [ ok ] spatial WHERE selected exactly the intersecting fixtures: {labels:?}"
                );
            } else {
                println!("  [FAIL] spatial WHERE returned {labels:?}, expected {want:?}");
                ctx.failures.push(format!(
                    "spatial WHERE returned {labels:?}, expected {want:?}"
                ));
            }
        }
        Err(e) => {
            println!("  [FAIL] spatial WHERE: {e}");
            ctx.failures.push(format!("spatial WHERE: {e}"));
        }
    }

    // The complementary case that pins hole semantics: a box lying STRICTLY
    // inside `poly_hole`'s interior ring must not match it. Without this, the
    // assertion above would pass on an implementation that ignored holes.
    let sql = format!(
        "SELECT properties->>'label'::String AS label FROM '{WORKSPACE}' \
         WHERE node_type = '{NODE_TYPE}' \
           AND ST_INTERSECTS({G}, ST_GEOMFROMGEOJSON('{{\"type\":\"Polygon\",\"coordinates\":[[[1.4,1.4],[1.6,1.4],[1.6,1.6],[1.4,1.6],[1.4,1.4]]]}}')) \
         ORDER BY label"
    );
    ctx.cov
        .record_sql(&sql, "a box inside an interior ring matches nothing");
    match ctx.sql(&sql).await {
        Ok(rows) => {
            let labels: Vec<&str> = rows
                .iter()
                .filter_map(|r| r.get("label").and_then(|v| v.as_str()))
                .collect();
            if labels.is_empty() {
                println!("  [ ok ] a box inside the interior ring matched nothing");
            } else {
                println!("  [FAIL] a box inside the interior ring matched {labels:?}");
                ctx.failures.push(format!(
                    "a box inside poly_hole's interior ring matched {labels:?}, expected none"
                ));
            }
        }
        Err(e) => ctx.failures.push(format!("interior-ring probe: {e}")),
    }

    // ST_DWITHIN as a filter — the index-eligible shape. Whether or not the
    // index is used, the ROWS must be right.
    let sql = format!(
        "SELECT properties->>'label'::String AS label FROM '{WORKSPACE}' \
         WHERE node_type = '{NODE_TYPE}' \
           AND ST_DWITHIN({G}, ST_POINT(8.5417, 47.3769), 100) ORDER BY label"
    );
    ctx.cov.record_sql(&sql, "ST_DWITHIN as a row filter");
    match ctx.sql(&sql).await {
        Ok(rows) => {
            let labels: Vec<&str> = rows
                .iter()
                .filter_map(|r| r.get("label").and_then(|v| v.as_str()))
                .collect();
            // Only the 4326 Zurich fixture is within 100 m of Zurich. The 3857
            // and UTM ones carry a different SRID, so they are either excluded or
            // raise a mismatch — either way they must not silently match.
            if labels.contains(&"zurich_4326") {
                println!("  [ ok ] ST_DWITHIN filter found the Zurich fixture: {labels:?}");
            } else {
                println!("  [FAIL] ST_DWITHIN filter returned {labels:?}");
                ctx.failures
                    .push(format!("ST_DWITHIN filter returned {labels:?}"));
            }
        }
        Err(e) => {
            println!("  [FAIL] ST_DWITHIN filter: {e}");
            ctx.failures.push(format!("ST_DWITHIN filter: {e}"));
        }
    }
}

/// A SQL UPDATE of a geometry property must take effect and be readable.
pub(super) async fn update(ctx: &mut Ctx) {
    let moved = r#"{"type":"Point","coordinates":[20,30]}"#;
    let sql = format!(
        "UPDATE '{WORKSPACE}' SET properties = '{}'::jsonb \
         WHERE path = '/pt' ",
        serde_json::json!({ "label": "pt", "kind": "Point", "g": serde_json::from_str::<Value>(moved).unwrap() })
            .to_string()
            .replace('\'', "''")
    );
    ctx.cov
        .record_sql(&sql, "SQL UPDATE of a geometry property");
    if let Err(e) = ctx.sql(&sql).await {
        println!("  [FAIL] UPDATE: {e}");
        ctx.failures.push(format!("geometry UPDATE: {e}"));
        return;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let read = row_sql("pt", &format!("ST_X({G})"));
    match ctx.sql(&read).await {
        Ok(rows) => {
            let x = rows
                .first()
                .and_then(|r| r.get("r"))
                .and_then(|v| v.as_f64());
            if x == Some(20.0) {
                println!("  [ ok ] SQL UPDATE moved the geometry and it reads back");
            } else {
                println!("  [FAIL] after UPDATE, ST_X is {x:?}, expected 20");
                ctx.failures
                    .push(format!("after geometry UPDATE ST_X is {x:?}, expected 20"));
            }
        }
        Err(e) => ctx.failures.push(format!("read after UPDATE: {e}")),
    }

    // Put it back so any later assertion sees the original corpus.
    let restore = format!(
        "UPDATE '{WORKSPACE}' SET properties = '{}'::jsonb WHERE path = '/pt'",
        serde_json::json!({
            "label": "pt", "kind": "Point",
            "g": serde_json::from_str::<Value>(geojson("pt")).unwrap()
        })
        .to_string()
        .replace('\'', "''")
    );
    let _ = ctx.sql(&restore).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}
