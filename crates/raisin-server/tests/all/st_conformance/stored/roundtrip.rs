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

use super::super::fixtures::{geojson, CORPUS};
use super::super::harness::Ctx;
use super::{row_sql, G};

pub(super) async fn round_trip(ctx: &mut Ctx) {
    for (label, kind, _) in CORPUS {
        // The GeoJSON `type` of the stored value, via ST_GEOMETRYTYPE.
        let want = match *kind {
            "Point3D" | "Zurich4326" | "Zurich3857" | "ZurichUTM32" => "ST_Point",
            other => &format!("ST_{other}").leak()[..],
        };
        let sql = row_sql(label, &format!("ST_GEOMETRYTYPE({G})"));
        ctx.cov.record_sql(&sql, "stored geometry round trip");
        match ctx.sql(&sql).await {
            Ok(rows) => {
                let got = rows
                    .first()
                    .and_then(|r| r.get("r"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("<missing>")
                    .to_string();
                if got == want {
                    println!("  [ ok ] {label} stored and read back as {want}");
                } else {
                    println!("  [FAIL] {label}: expected {want}, read back {got}");
                    ctx.failures
                        .push(format!("stored {label}: expected {want}, got {got}"));
                }
            }
            Err(e) => {
                println!("  [FAIL] {label} round trip: {e}");
                ctx.failures.push(format!("stored {label} round trip: {e}"));
            }
        }
    }

    // The stored bytes are byte-identical to what was written (2-D fixtures
    // serialize with no `srid` member and no altitude, so the text matches).
    let sql = row_sql("pt", "ST_ASGEOJSON(CAST(properties->>'g' AS GEOMETRY))");
    ctx.cov.record_sql(&sql, "stored geometry is byte-stable");
    match ctx.sql(&sql).await {
        Ok(rows) => {
            let got = rows
                .first()
                .and_then(|r| r.get("r"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            // Compare parsed JSON so 1 vs 1.0 formatting is not the subject.
            let a: Value = serde_json::from_str(&got).unwrap_or(Value::Null);
            let b: Value = serde_json::from_str(geojson("pt")).unwrap();
            if numeric_eq(&a, &b) {
                println!("  [ ok ] stored Point round-trips its coordinates exactly");
            } else {
                println!("  [FAIL] stored Point changed: wrote {b}, read {a}");
                ctx.failures
                    .push(format!("stored Point changed: wrote {b}, read {a}"));
            }
        }
        Err(e) => ctx.failures.push(format!("stored Point asgeojson: {e}")),
    }
}

/// JSON equality that treats 1 and 1.0 as the same number.
fn numeric_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-12,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| numeric_eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).map(|w| numeric_eq(v, w)).unwrap_or(false))
        }
        _ => a == b,
    }
}

/// Measurement, accessor and processing functions applied to stored geometry.
pub(super) async fn functions(ctx: &mut Ctx) {
    let cases: Vec<(&str, &str, &str, Box<dyn Fn(&Value) -> bool>)> = vec![
        (
            "pt",
            "ST_X over stored data",
            "ST_X(CAST(properties->>'g' AS GEOMETRY))",
            Box::new(|v| v.as_f64().map(|x| (x - 1.0).abs() < 1e-9).unwrap_or(false)),
        ),
        (
            "ls",
            "ST_LENGTH over stored data",
            "ST_LENGTH(CAST(properties->>'g' AS GEOMETRY))",
            // Two 2-degree legs = 4 spherical degrees = 444780 m.
            Box::new(|v| {
                v.as_f64()
                    .map(|x| (x - 444_780.0).abs() < 2_000.0)
                    .unwrap_or(false)
            }),
        ),
        (
            "poly_hole",
            "ST_AREA over stored data (with a same-wound hole)",
            "ST_AREA(CAST(properties->>'g' AS GEOMETRY))",
            // The exact ellipsoidal area of the lat/lon rectangle 0..4 x 0..4 is
            // 196_789_484_713 m^2 and of the 1..2 x 1..2 hole is 12_304_814_950,
            // so shell - hole = 184_484_669_763. The geodesic polygon through the
            // same corners is a shade larger because its edges are geodesics
            // rather than parallels of latitude, so a 1e-3 relative window is the
            // right bar. (Before the winding fix this returned -5.1e14.)
            Box::new(|v| {
                v.as_f64()
                    .map(|a| ((a - 184_484_669_763.0) / 184_484_669_763.0f64).abs() < 1e-3)
                    .unwrap_or(false)
            }),
        ),
        (
            "mpoly",
            "ST_NUMGEOMETRIES over stored data",
            "ST_NUMGEOMETRIES(CAST(properties->>'g' AS GEOMETRY))",
            Box::new(|v| v.as_i64() == Some(2)),
        ),
        (
            "gc",
            "ST_NUMGEOMETRIES of a stored GeometryCollection",
            "ST_NUMGEOMETRIES(CAST(properties->>'g' AS GEOMETRY))",
            Box::new(|v| v.as_i64() == Some(3)),
        ),
        (
            "pt3d",
            "ST_Z over stored 3-D data",
            "ST_Z(CAST(properties->>'g' AS GEOMETRY))",
            Box::new(|v| v.as_f64() == Some(250.0)),
        ),
        (
            "pt3d",
            "ST_NDIMS over stored 3-D data",
            "ST_NDIMS(CAST(properties->>'g' AS GEOMETRY))",
            Box::new(|v| v.as_i64() == Some(3)),
        ),
        (
            "mpt",
            "ST_CONVEXHULL over stored data",
            "ST_GEOMETRYTYPE(ST_CONVEXHULL(CAST(properties->>'g' AS GEOMETRY)))",
            Box::new(|v| v.as_str() == Some("ST_Polygon")),
        ),
        (
            "ls",
            "ST_BUFFER over stored data",
            "ST_AREA(ST_BUFFER(CAST(properties->>'g' AS GEOMETRY), 1000))",
            Box::new(|v| v.as_f64().map(|a| a > 0.0).unwrap_or(false)),
        ),
        (
            "poly_hole",
            "ST_ISVALID over stored data",
            "ST_ISVALID(CAST(properties->>'g' AS GEOMETRY))",
            Box::new(|v| v == &Value::Bool(true)),
        ),
    ];

    for (label, what, expr_sql, check) in cases {
        let sql = row_sql(label, expr_sql);
        ctx.cov.record_sql(&sql, what);
        match ctx.sql(&sql).await {
            Ok(rows) => {
                let got = rows
                    .first()
                    .and_then(|r| r.get("r").cloned())
                    .unwrap_or(Value::Null);
                if check(&got) {
                    println!("  [ ok ] {what}  ({got})");
                } else {
                    println!("  [FAIL] {what}: unexpected {got}");
                    ctx.failures.push(format!("{what}: unexpected {got}"));
                }
            }
            Err(e) => {
                println!("  [FAIL] {what}: {e}");
                ctx.failures.push(format!("{what}: {e}"));
            }
        }
    }
}
