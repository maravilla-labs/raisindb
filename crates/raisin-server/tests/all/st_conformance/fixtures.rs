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

//! The geometry corpus: one stored node per GeoJSON type, written over **SQL
//! INSERT** so the suite proves the SQL write path as a side effect.
//!
//! Coordinates are deliberately small integers around the origin. Two reasons:
//! a 1°×1° figure at the equator has hand-checkable geodesic dimensions
//! (a degree of latitude is 110574.4 m, a degree of longitude at the equator is
//! 111319.5 m on WGS84), and keeping shapes near lon 0 keeps them inside a single
//! UTM zone so the projected paths in `ST_DISTANCE` / `ST_BUFFER` are not fighting
//! zone-edge distortion at the same time as the thing under test.

use super::harness::{Ctx, NODE_TYPE, WORKSPACE};

/// `(label, kind, geojson)` — `kind` is the GeoJSON type, used to select rows.
pub const CORPUS: &[(&str, &str, &str)] = &[
    ("pt", "Point", r#"{"type":"Point","coordinates":[1,1]}"#),
    (
        "ls",
        "LineString",
        r#"{"type":"LineString","coordinates":[[0,0],[2,0],[2,2]]}"#,
    ),
    // A unit square with a square hole. The hole is wound the SAME way as the
    // shell on purpose: RFC 7946 recommends the right-hand rule but requires
    // parsers not to reject other winding, OGC shapefiles wind shells clockwise,
    // and `ST_AREA` returned the surface area of the Earth for this input before
    // this suite caught it. See `measures::area_is_winding_independent`.
    (
        "poly_hole",
        "Polygon",
        r#"{"type":"Polygon","coordinates":[[[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,2],[1,1]]]}"#,
    ),
    (
        "mpt",
        "MultiPoint",
        r#"{"type":"MultiPoint","coordinates":[[0,0],[3,1],[1,4]]}"#,
    ),
    (
        "mls",
        "MultiLineString",
        r#"{"type":"MultiLineString","coordinates":[[[0,0],[1,0]],[[0,2],[1,2]]]}"#,
    ),
    (
        "mpoly",
        "MultiPolygon",
        r#"{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,1],[0,0]]],[[[5,5],[6,5],[6,6],[5,6],[5,5]]]]}"#,
    ),
    (
        "gc",
        "GeometryCollection",
        r#"{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[1,1]},{"type":"LineString","coordinates":[[0,0],[2,0]]},{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1],[0,0]]]}]}"#,
    ),
    // 3-D: a Point carrying an altitude, for ST_Z / ST_NDIMS / ST_3DDISTANCE.
    (
        "pt3d",
        "Point3D",
        r#"{"type":"Point","coordinates":[1,1,250]}"#,
    ),
    // Explicitly labelled non-4326 geometries, for the CRS assertions. These are
    // the SAME PLACE as `pt_zurich` below, expressed in three CRSs — see
    // `crs::same_place_three_ways`.
    (
        "zurich_4326",
        "Zurich4326",
        r#"{"type":"Point","coordinates":[8.5417,47.3769]}"#,
    ),
    // EPSG:3857 from the closed form of Pseudo-Mercator, and EPSG:32632 from an
    // independent Krüger series — both derived outside this codebase. See the
    // module docs in `crs.rs`.
    (
        "zurich_3857",
        "Zurich3857",
        r#"{"type":"Point","coordinates":[950857.6945,6003812.2049],"srid":3857}"#,
    ),
    (
        "zurich_utm32",
        "ZurichUTM32",
        r#"{"type":"Point","coordinates":[465403.284,5247150.839],"srid":32632}"#,
    ),
];

/// GeoJSON literal for a corpus entry, by label. Panics on an unknown label so a
/// typo is a compile-adjacent failure rather than a silently skipped assertion.
pub fn geojson(label: &str) -> &'static str {
    CORPUS
        .iter()
        .find(|(l, _, _)| *l == label)
        .map(|(_, _, g)| *g)
        .unwrap_or_else(|| panic!("no corpus fixture labelled {label:?}"))
}

/// A `ST_GEOMFROMGEOJSON('...')` expression for a corpus entry.
pub fn expr(label: &str) -> String {
    format!("ST_GEOMFROMGEOJSON('{}')", geojson(label))
}

/// Insert the whole corpus via SQL INSERT.
///
/// `path` is required by the workspace DML contract; omitting it fails with
/// "Required column 'path' is missing".
pub async fn insert_all(ctx: &mut Ctx) {
    println!(
        "\n--- fixtures: inserting {} nodes via SQL ---",
        CORPUS.len()
    );
    for (label, kind, geo) in CORPUS {
        let props = serde_json::json!({ "label": label, "kind": kind, "g": serde_json::from_str::<serde_json::Value>(geo).expect("fixture is valid JSON") });
        let sql = format!(
            "INSERT INTO '{WORKSPACE}' (path, name, node_type, properties) \
             VALUES ('/{label}', '{label}', '{NODE_TYPE}', '{}'::jsonb)",
            props.to_string().replace('\'', "''")
        );
        match ctx.sql(&sql).await {
            Ok(_) => println!("  [ ok ] inserted {label} ({kind})"),
            Err(e) => {
                println!("  [FAIL] insert {label}: {e}");
                ctx.failures.push(format!("fixture insert {label}: {e}"));
            }
        }
    }
    // Give indexing a moment to settle before the stored-data assertions.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

/// A geometry read back out of a stored node, as an ST_* argument.
///
/// `CAST(properties->>'g' AS GEOMETRY)` is the form that works: `properties->>'g'`
/// yields TEXT and no ST_* signature accepts `TEXT?`, while
/// `ST_GEOMFROMGEOJSON(properties->>'g'::String)` yields NULL for a property the
/// nodetype declares as `Geometry`.
pub fn stored(label: &str) -> String {
    let _ = geojson(label); // fail fast on a bad label
    format!("CAST(properties->>'g' AS GEOMETRY)")
}

/// `FROM ... WHERE label = '<label>'` clause selecting exactly one fixture row.
pub fn where_label(label: &str) -> String {
    let _ = geojson(label);
    format!(
        "FROM '{WORKSPACE}' WHERE node_type = '{NODE_TYPE}' AND properties->>'label'::String = '{label}'"
    )
}
