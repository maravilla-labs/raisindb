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

//! The third dimension, read off the JSON representation.
//!
//! `geo_types::Coord` has no Z, so altitude never enters the `geo` pipeline.
//! Every Z-aware SQL function therefore reads it from here (or from
//! [`Geom::z_range`](crate::Geom::z_range)) rather than from a `geo` geometry.
//!
//! All the *other* ST_\* functions ignore Z, exactly as PostGIS's 2-D predicates
//! do. That is stated once, here and in the docs, rather than repeated as a
//! caveat on forty functions.

use serde_json::Value;

/// The altitude of a Point, or `None`.
///
/// `None` for a 2-D Point **and** for any non-Point, matching PostGIS `ST_Z`,
/// which returns NULL rather than erroring.
pub fn z_of_point(v: &Value) -> Option<f64> {
    if v.get("type").and_then(Value::as_str) != Some("Point") {
        return None;
    }
    let coords = v.get("coordinates")?.as_array()?;
    coords.get(2)?.as_f64()
}

/// 2 or 3 — 3 if *any* position in the geometry carries an altitude.
pub fn ndims(v: &Value) -> u8 {
    if z_range_of_json(v).is_some() {
        3
    } else {
        2
    }
}

/// The `(min, max)` altitude across every position that has one.
///
/// Walks the raw JSON rather than the model so that it works on values the SQL
/// evaluator holds, before (or without) any conversion.
pub fn z_range_of_json(v: &Value) -> Option<(f64, f64)> {
    let mut range: Option<(f64, f64)> = None;
    walk_positions(v, &mut |ords| {
        if let Some(z) = ords.get(2).and_then(Value::as_f64) {
            range = Some(match range {
                None => (z, z),
                Some((lo, hi)) => (lo.min(z), hi.max(z)),
            });
        }
    });
    range
}

/// Drop every altitude, producing a strictly 2-D geometry.
pub fn force_2d(v: &Value) -> Value {
    map_positions(v, &|ords| ords.iter().take(2).cloned().collect())
}

/// Give every 2-D position the supplied altitude, leaving existing ones alone.
///
/// Matches PostGIS `ST_Force3D`, which fills in the missing ordinate rather than
/// overwriting a present one.
pub fn force_3d(v: &Value, z: f64) -> Value {
    let zv = Value::from(z);
    map_positions(v, &|ords| {
        if ords.len() >= 3 {
            ords.to_vec()
        } else {
            let mut out = ords.to_vec();
            out.truncate(2);
            out.push(zv.clone());
            out
        }
    })
}

/// The signed vertical gap between two altitude intervals: `0.0` when they
/// overlap, otherwise the distance between the nearer endpoints.
///
/// This is the vertical leg of `ST_3DDISTANCE`. Using the interval gap rather
/// than a centroid difference means a tall building and a point inside its
/// altitude band are treated as vertically coincident, which is the useful
/// answer for "how far apart are these things".
pub fn z_gap(a: Option<(f64, f64)>, b: Option<(f64, f64)>) -> Option<f64> {
    let (a_lo, a_hi) = a?;
    let (b_lo, b_hi) = b?;
    if a_hi < b_lo {
        Some(b_lo - a_hi)
    } else if b_hi < a_lo {
        Some(a_lo - b_hi)
    } else {
        Some(0.0)
    }
}

// --- traversal ---------------------------------------------------------------

/// Call `f` with every position (an array of numbers) in a GeoJSON value.
///
/// A position is recognised structurally: an array whose elements are all
/// numbers. That is unambiguous inside a GeoJSON `coordinates` member at any
/// nesting depth, and it means one traversal covers all seven types plus nested
/// collections.
fn walk_positions(v: &Value, f: &mut impl FnMut(&[Value])) {
    match v {
        Value::Array(items) => {
            if !items.is_empty() && items.iter().all(Value::is_number) {
                f(items);
            } else {
                for item in items {
                    walk_positions(item, f);
                }
            }
        }
        Value::Object(map) => {
            if let Some(c) = map.get("coordinates") {
                walk_positions(c, f);
            }
            if let Some(g) = map.get("geometries") {
                walk_positions(g, f);
            }
            // Features and FeatureCollections, so ST_FORCE2D works on them too.
            if let Some(g) = map.get("geometry") {
                walk_positions(g, f);
            }
            if let Some(g) = map.get("features") {
                walk_positions(g, f);
            }
        }
        _ => {}
    }
}

/// Structure-preserving rewrite of every position.
fn map_positions(v: &Value, f: &dyn Fn(&[Value]) -> Vec<Value>) -> Value {
    match v {
        Value::Array(items) => {
            if !items.is_empty() && items.iter().all(Value::is_number) {
                Value::Array(f(items))
            } else {
                Value::Array(items.iter().map(|i| map_positions(i, f)).collect())
            }
        }
        Value::Object(map) => {
            let mut out = map.clone();
            for key in ["coordinates", "geometries", "geometry", "features"] {
                if let Some(child) = map.get(key) {
                    out.insert(key.to_string(), map_positions(child, f));
                }
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn z_of_point_is_none_for_flat_points_and_for_non_points() {
        assert_eq!(
            z_of_point(&json!({"type":"Point","coordinates":[1,2,3.5]})),
            Some(3.5)
        );
        assert_eq!(
            z_of_point(&json!({"type":"Point","coordinates":[1,2]})),
            None
        );
        assert_eq!(
            z_of_point(&json!({"type":"LineString","coordinates":[[1,2,3]]})),
            None,
            "PostGIS ST_Z returns NULL for a non-Point"
        );
        assert_eq!(z_of_point(&json!({"type":"Point"})), None);
    }

    #[test]
    fn ndims_and_z_range_walk_nested_geometries() {
        let flat = json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]});
        assert_eq!(ndims(&flat), 2);
        assert_eq!(z_range_of_json(&flat), None);

        let mixed = json!({"type":"GeometryCollection","geometries":[
            {"type":"Point","coordinates":[0,0]},
            {"type":"MultiPolygon","coordinates":[[[[0,0,12.0],[1,0,-4.0],[1,1],[0,0,12.0]]]]}
        ]});
        assert_eq!(ndims(&mixed), 3);
        assert_eq!(z_range_of_json(&mixed), Some((-4.0, 12.0)));
    }

    #[test]
    fn force_2d_strips_z_at_every_depth_and_keeps_the_shape() {
        let v = json!({"type":"MultiLineString","coordinates":[
            [[0,0,1.0],[1,1,2.0]],
            [[2,2],[3,3,4.0]]
        ]});
        let flat = force_2d(&v);
        assert_eq!(
            flat,
            json!({"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]})
        );
        assert_eq!(ndims(&flat), 2);
    }

    #[test]
    fn force_3d_fills_only_the_missing_ordinates() {
        let v = json!({"type":"LineString","coordinates":[[0,0],[1,1,9.0]]});
        let up = force_3d(&v, 5.0);
        assert_eq!(
            up,
            json!({"type":"LineString","coordinates":[[0,0,5.0],[1,1,9.0]]})
        );
        assert_eq!(z_range_of_json(&up), Some((5.0, 9.0)));
    }

    #[test]
    fn force_2d_and_force_3d_leave_unrelated_members_alone() {
        let v = json!({"type":"Point","coordinates":[1,2,3.0],"srid":2056});
        assert_eq!(force_2d(&v)["srid"], 2056);
        assert_eq!(
            force_3d(&force_2d(&v), 7.0)["coordinates"],
            json!([1, 2, 7.0])
        );
    }

    #[test]
    fn empty_coordinate_arrays_are_not_mistaken_for_positions() {
        let v = json!({"type":"MultiPoint","coordinates":[]});
        assert_eq!(force_2d(&v), v);
        assert_eq!(z_range_of_json(&v), None);
    }

    #[test]
    fn features_are_walked_too() {
        let v = json!({
            "type":"Feature",
            "properties":{"floor":"L2"},
            "geometry":{"type":"Point","coordinates":[1,2,8.0]}
        });
        assert_eq!(z_range_of_json(&v), Some((8.0, 8.0)));
        assert_eq!(force_2d(&v)["geometry"]["coordinates"], json!([1, 2]));
        assert_eq!(force_2d(&v)["properties"]["floor"], "L2");
    }

    #[test]
    fn z_gap_is_zero_when_the_intervals_overlap() {
        assert_eq!(z_gap(Some((0.0, 10.0)), Some((5.0, 20.0))), Some(0.0));
        assert_eq!(z_gap(Some((0.0, 10.0)), Some((10.0, 20.0))), Some(0.0));
        assert_eq!(z_gap(Some((0.0, 10.0)), Some((14.0, 20.0))), Some(4.0));
        assert_eq!(z_gap(Some((30.0, 40.0)), Some((0.0, 10.0))), Some(20.0));
        // A 2-D operand has no vertical position, so there is no 3-D answer.
        assert_eq!(z_gap(None, Some((0.0, 1.0))), None);
        assert_eq!(z_gap(Some((0.0, 1.0)), None), None);
    }
}
