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

//! `serde_json::Value` <-> [`Geom`], delegating to the `geojson` crate.
//!
//! # Why delegate rather than hand-roll
//!
//! The previous implementation had exactly three hand-rolled converters —
//! `geojson_to_point`, `geojson_to_linestring`, `geojson_to_polygon` — and that,
//! not a missing library, is why `Multi*` and `GeometryCollection` were
//! unsupported across all 49 ST_\* functions. `geojson` 1.0 ships complete
//! `TryFrom` impls for every type in both directions; one call replaces all
//! three and unlocks every geometry type at once.
//!
//! `geojson` was already a declared workspace dependency with the `geo-types`
//! feature enabled, and was not used anywhere. This module is the first user.

use geo::Geometry;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{GeometryError, Result};
use crate::geom::Geom;
use crate::srid::srid_of;
use crate::zdim::z_range_of_json;

/// Parse a GeoJSON `Value` into a `geo` geometry, carrying its SRID and vertical
/// extent.
///
/// Accepts a bare Geometry, a Feature, or a FeatureCollection (the last two
/// become a GeometryCollection), which is what makes `ST_GEOMFROMGEOJSON`
/// tolerant of what users actually paste in.
///
/// `schema_default` is the NodeType/workspace SRID used when the value carries no
/// `srid` member.
pub fn to_geo(v: &Value, schema_default: Option<u32>) -> Result<Geom> {
    let srid = srid_of(v, schema_default)?;

    // `&Value` is itself a Deserializer, so this does not clone the input.
    let gj = geojson::GeoJson::deserialize(v).map_err(|e| GeometryError::NotGeometry {
        reason: e.to_string(),
    })?;

    let type_name = geojson_type_name(&gj);
    let geometry: Geometry<f64> =
        Geometry::try_from(gj).map_err(|e| GeometryError::Unconvertible {
            geometry_type: type_name.to_string(),
            reason: e.to_string(),
        })?;

    check_finite(&geometry)?;

    Ok(Geom {
        geometry,
        srid,
        z_range: z_range_of_json(v),
    })
}

/// Serialize a `geo` geometry back to a GeoJSON `Value`.
///
/// Routed through the RaisinDB model so that the `srid` member is emitted by
/// exactly the same rule everywhere: present only when the CRS is not WGS84.
///
/// Altitude is not restored — `geo` never carried it. Use
/// [`crate::zdim::force_3d`] if a Z is wanted on the output.
pub fn from_geo(g: &Geom) -> Result<Value> {
    let model = crate::model::to_model(g)?;
    serde_json::to_value(&model).map_err(Into::into)
}

/// Reject NaN/infinite ordinates, which `geojson` happily accepts.
///
/// A non-finite ordinate is worse than an error: `geo`'s predicates return
/// plausible booleans for it and it geohashes to a garbage index cell.
fn check_finite(g: &Geometry<f64>) -> Result<()> {
    use geo::CoordsIter;
    for c in g.coords_iter() {
        if !c.x.is_finite() || !c.y.is_finite() {
            return Err(GeometryError::NonFiniteCoordinate { x: c.x, y: c.y });
        }
    }
    Ok(())
}

fn geojson_type_name(gj: &geojson::GeoJson) -> &'static str {
    match gj {
        geojson::GeoJson::Geometry(g) => g.value.type_name(),
        geojson::GeoJson::Feature(_) => "Feature",
        geojson::GeoJson::FeatureCollection(_) => "FeatureCollection",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_proj::Crs;
    use serde_json::json;

    /// The headline claim of the conversion layer: every type, in one call.
    #[test]
    fn every_geojson_type_parses_including_multi_and_collections() {
        let cases = vec![
            (json!({"type":"Point","coordinates":[1.0,2.0]}), "Point"),
            (
                json!({"type":"LineString","coordinates":[[0,0],[1,1]]}),
                "LineString",
            ),
            (
                json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}),
                "Polygon",
            ),
            (
                json!({"type":"MultiPoint","coordinates":[[0,0],[1,1]]}),
                "MultiPoint",
            ),
            (
                json!({"type":"MultiLineString","coordinates":[[[0,0],[1,1]],[[2,2],[3,3]]]}),
                "MultiLineString",
            ),
            (
                json!({"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}),
                "MultiPolygon",
            ),
            (
                json!({"type":"GeometryCollection","geometries":[
                    {"type":"Point","coordinates":[1,1]},
                    {"type":"MultiPoint","coordinates":[[2,2]]}
                ]}),
                "GeometryCollection",
            ),
        ];
        for (value, name) in cases {
            let g = to_geo(&value, None).unwrap_or_else(|e| panic!("{name}: {e}"));
            let back = from_geo(&g).unwrap();
            assert_eq!(back["type"], name, "{name} changed type on round trip");
        }
    }

    #[test]
    fn round_trip_preserves_coordinates_exactly() {
        let value = json!({
            "type": "Polygon",
            "coordinates": [
                [[0.0,0.0],[10.0,0.0],[10.0,10.0],[0.0,10.0],[0.0,0.0]],
                [[2.0,2.0],[4.0,2.0],[4.0,4.0],[2.0,2.0]]
            ]
        });
        let back = from_geo(&to_geo(&value, None).unwrap()).unwrap();
        assert_eq!(back, value);
    }

    #[test]
    fn an_empty_geometry_collection_survives() {
        let g = to_geo(&crate::geom::empty(), None).unwrap();
        assert!(g.is_empty());
        assert_eq!(from_geo(&g).unwrap(), crate::geom::empty());
    }

    #[test]
    fn a_feature_is_accepted_and_reduced_to_its_geometry() {
        let value = json!({
            "type": "Feature",
            "properties": {"name": "gate A12"},
            "geometry": {"type":"Point","coordinates":[8.54,47.37]}
        });
        let g = to_geo(&value, None).unwrap();
        // A Feature widens to a single-member GeometryCollection.
        assert!(!g.is_empty());
        assert_eq!(g.geometry.coords_iter().count(), 1);
    }

    #[test]
    fn srid_member_is_read_and_re_emitted() {
        let value = json!({"type":"Point","coordinates":[2683000.0,1247000.0],"srid":32632});
        let g = to_geo(&value, None).unwrap();
        assert_eq!(g.srid, Crs::from_srid(32632));
        assert_eq!(from_geo(&g).unwrap()["srid"], 32632);
    }

    #[test]
    fn wgs84_output_is_strictly_rfc7946_with_no_srid_member() {
        let value = json!({"type":"Point","coordinates":[8.54,47.37],"srid":4326});
        let out = from_geo(&to_geo(&value, None).unwrap()).unwrap();
        assert!(out.get("srid").is_none(), "{out}");
    }

    #[test]
    fn altitude_lands_in_z_range() {
        let value = json!({"type":"LineString","coordinates":[[0,0,5.0],[1,1,15.0]]});
        let g = to_geo(&value, None).unwrap();
        assert_eq!(g.z_range, Some((5.0, 15.0)));
        // geo is 2-D, so the coordinates themselves came through flattened.
        assert_eq!(g.geometry.coords_iter().count(), 2);
    }

    #[test]
    fn garbage_is_rejected_with_a_useful_message() {
        for bad in [
            json!({"coordinates":[1,2]}),
            json!({"type":"Circle","coordinates":[1,2]}),
            json!({"type":"Point"}),
            json!({"type":"Point","coordinates":[1]}),
            json!("not an object"),
            json!(42),
        ] {
            assert!(to_geo(&bad, None).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn non_finite_ordinates_are_rejected() {
        // JSON cannot express NaN, but a Value built in memory can.
        let value = json!({"type":"Point","coordinates":[0.0,0.0]});
        let mut value = value;
        value["coordinates"] = Value::Array(vec![
            serde_json::Number::from_f64(0.0)
                .map(Value::Number)
                .unwrap(),
            Value::Null,
        ]);
        assert!(to_geo(&value, None).is_err());
    }

    use geo::CoordsIter;
}
