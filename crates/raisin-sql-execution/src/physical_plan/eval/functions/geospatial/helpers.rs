//! Helper functions for geospatial operations
//!
//! Provides utilities for converting between GeoJSON (serde_json::Value)
//! and geo crate types, plus coordinate extraction helpers.

use geo::{Coord, LineString, Point, Polygon};
use raisin_error::Error;
use serde_json::Value;

/// Convert a serde_json::Value (GeoJSON) to a geo::Point
///
/// Expects: {"type": "Point", "coordinates": [lon, lat]}
pub fn geojson_to_point(value: &Value) -> Result<Point, Error> {
    let geom_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("GeoJSON missing 'type' field".to_string()))?;

    if geom_type != "Point" {
        return Err(Error::Validation(format!(
            "Expected Point geometry, got {}",
            geom_type
        )));
    }

    let coords = value
        .get("coordinates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            Error::Validation("GeoJSON Point missing 'coordinates' array".to_string())
        })?;

    if coords.len() < 2 {
        return Err(Error::Validation(
            "Point coordinates must have at least [lon, lat]".to_string(),
        ));
    }

    let lon = coords[0]
        .as_f64()
        .ok_or_else(|| Error::Validation("Invalid longitude value".to_string()))?;
    let lat = coords[1]
        .as_f64()
        .ok_or_else(|| Error::Validation("Invalid latitude value".to_string()))?;

    Ok(Point::new(lon, lat))
}

/// Convert a serde_json::Value (GeoJSON) to a geo::LineString
///
/// Expects: {"type": "LineString", "coordinates": [[lon, lat], [lon, lat], ...]}
pub fn geojson_to_linestring(value: &Value) -> Result<LineString, Error> {
    let geom_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("GeoJSON missing 'type' field".to_string()))?;

    if geom_type != "LineString" {
        return Err(Error::Validation(format!(
            "Expected LineString geometry, got {}",
            geom_type
        )));
    }

    let coords = value
        .get("coordinates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            Error::Validation("GeoJSON LineString missing 'coordinates' array".to_string())
        })?;

    let points: Result<Vec<Coord>, Error> = coords
        .iter()
        .map(|c| {
            let arr = c
                .as_array()
                .ok_or_else(|| Error::Validation("Invalid coordinate pair".to_string()))?;
            if arr.len() < 2 {
                return Err(Error::Validation(
                    "Coordinate must have [lon, lat]".to_string(),
                ));
            }
            let lon = arr[0]
                .as_f64()
                .ok_or_else(|| Error::Validation("Invalid longitude".to_string()))?;
            let lat = arr[1]
                .as_f64()
                .ok_or_else(|| Error::Validation("Invalid latitude".to_string()))?;
            Ok(Coord { x: lon, y: lat })
        })
        .collect();

    Ok(LineString::new(points?))
}

/// Convert a serde_json::Value (GeoJSON) to a geo::Polygon
///
/// Expects: {"type": "Polygon", "coordinates": [[[lon, lat], ...], ...]}
pub fn geojson_to_polygon(value: &Value) -> Result<Polygon, Error> {
    let geom_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("GeoJSON missing 'type' field".to_string()))?;

    if geom_type != "Polygon" {
        return Err(Error::Validation(format!(
            "Expected Polygon geometry, got {}",
            geom_type
        )));
    }

    let rings = value
        .get("coordinates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            Error::Validation("GeoJSON Polygon missing 'coordinates' array".to_string())
        })?;

    if rings.is_empty() {
        return Err(Error::Validation(
            "Polygon must have at least one ring".to_string(),
        ));
    }

    // Parse exterior ring
    let exterior_coords = parse_ring(&rings[0])?;
    let exterior = LineString::new(exterior_coords);

    // Parse interior rings (holes)
    let interiors: Result<Vec<LineString>, Error> = rings
        .iter()
        .skip(1)
        .map(|ring| {
            let coords = parse_ring(ring)?;
            Ok(LineString::new(coords))
        })
        .collect();

    Ok(Polygon::new(exterior, interiors?))
}

/// Parse a ring (array of coordinate arrays) into a Vec<Coord>
fn parse_ring(ring: &Value) -> Result<Vec<Coord>, Error> {
    let coords = ring
        .as_array()
        .ok_or_else(|| Error::Validation("Ring must be an array".to_string()))?;

    coords
        .iter()
        .map(|c| {
            let arr = c
                .as_array()
                .ok_or_else(|| Error::Validation("Invalid coordinate pair".to_string()))?;
            if arr.len() < 2 {
                return Err(Error::Validation(
                    "Coordinate must have [lon, lat]".to_string(),
                ));
            }
            let lon = arr[0]
                .as_f64()
                .ok_or_else(|| Error::Validation("Invalid longitude".to_string()))?;
            let lat = arr[1]
                .as_f64()
                .ok_or_else(|| Error::Validation("Invalid latitude".to_string()))?;
            Ok(Coord { x: lon, y: lat })
        })
        .collect()
}

/// Get the geometry type from a GeoJSON value
pub fn get_geometry_type(value: &Value) -> Result<&str, Error> {
    value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("GeoJSON missing 'type' field".to_string()))
}

/// Create a GeoJSON Point from lon/lat
pub fn point_to_geojson(lon: f64, lat: f64) -> Value {
    serde_json::json!({
        "type": "Point",
        "coordinates": [lon, lat]
    })
}

/// Extract centroid coordinates [lon, lat] from any geometry
pub fn get_centroid(value: &Value) -> Result<Point, Error> {
    let geom_type = get_geometry_type(value)?;

    match geom_type {
        "Point" => geojson_to_point(value),
        "LineString" => {
            let line = geojson_to_linestring(value)?;
            use geo::Centroid;
            line.centroid().ok_or_else(|| {
                Error::Validation("Cannot compute centroid of empty LineString".to_string())
            })
        }
        "Polygon" => {
            let polygon = geojson_to_polygon(value)?;
            use geo::Centroid;
            polygon.centroid().ok_or_else(|| {
                Error::Validation("Cannot compute centroid of empty Polygon".to_string())
            })
        }
        other => Err(Error::Validation(format!(
            "Centroid not supported for geometry type: {}",
            other
        ))),
    }
}
