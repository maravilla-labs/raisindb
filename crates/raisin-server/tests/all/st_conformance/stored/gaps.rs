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

use super::super::harness::{Ctx, NODE_TYPE, WORKSPACE};
use super::{row_sql, G};

/// A property the nodetype declares as `Geometry` must reject malformed GeoJSON
/// LOUDLY rather than storing an unindexed pseudo-geometry.
///
/// This is the top silent-data-loss risk for SQL writes specifically: a
/// mistyped literal infers to `PropertyValue::Object`, is never spatially
/// indexed, and nothing complains.
pub(super) async fn malformed(ctx: &mut Ctx) {
    let bad = serde_json::json!({
        "label": "bad", "kind": "Bad",
        "g": { "type": "Point", "coordinates": "not-an-array" }
    })
    .to_string()
    .replace('\'', "''");
    let sql = format!(
        "INSERT INTO '{WORKSPACE}' (path, name, node_type, properties) \
         VALUES ('/bad', 'bad', '{NODE_TYPE}', '{bad}'::jsonb)"
    );
    match ctx.sql(&sql).await {
        Err(e) => println!(
            "  [ ok ] malformed geometry rejected at write: {}",
            first_line(&e)
        ),
        Ok(_) => {
            // It stored. Is it at least visible as a geometry afterwards? If not,
            // this is the silent-loss case: the value infers to
            // `PropertyValue::Object`, is never spatially indexed, and nothing
            // complains at any point.
            let probe = row_sql("bad", &format!("ST_GEOMETRYTYPE({G})"));
            let got = ctx
                .sql(&probe)
                .await
                .ok()
                .and_then(|rows| rows.first().and_then(|r| r.get("r").cloned()));
            ctx.gap(format!(
                "malformed GeoJSON was ACCEPTED for a property the nodetype declares as \
                 Geometry; reading it back as a geometry gives {got:?}. Spec decision 11(c) \
                 requires the write to fail loudly — otherwise the value becomes a \
                 PropertyValue::Object that is never spatially indexed, with no error \
                 anywhere. Not an ST_* defect; reported separately."
            ));
        }
    }

    // The SRID-mismatch rule is enforced by ST_DISTANCE but not, at the time of
    // writing, by the topological predicates. Probe rather than assert, so this
    // reports the real state instead of pinning either behaviour.
    let mixed = format!(
        "SELECT ST_INTERSECTS({}, {}) AS r",
        r#"ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[1,2],"srid":4326}')"#,
        r#"ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[1,2],"srid":3857}')"#
    );
    match ctx.sql(&mixed).await {
        Err(_) => println!("  [ ok ] ST_INTERSECTS enforces the SRID mismatch rule"),
        Ok(_) => ctx.gap(
            "ST_INTERSECTS does NOT enforce the SRID mismatch rule that ST_DISTANCE does \
             (an operation across two different explicit SRIDs should error and suggest \
             ST_TRANSFORM). The shared helper exists; the predicates do not call it."
                .to_string(),
        ),
    }

    // ST_POINT validates its two arguments as lon/lat unconditionally, so the
    // idiomatic PostGIS form ST_SetSRID(ST_MakePoint(x, y), <projected srid>) is
    // impossible for projected coordinates. Probe and report.
    if ctx
        .sql("SELECT ST_X(ST_SETSRID(ST_POINT(2683000, 1247000), 2056)) AS r")
        .await
        .is_err()
    {
        ctx.gap(
            "ST_POINT range-checks its arguments as longitude/latitude even when the \
             result is immediately labelled with a projected SRID, so \
             ST_SETSRID(ST_POINT(easting, northing), <projected>) — the idiomatic PostGIS \
             form — is rejected. Projected points must be built with ST_GEOMFROMGEOJSON."
                .to_string(),
        );
    }
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).chars().take(160).collect()
}
