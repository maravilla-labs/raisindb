//! Geometry centroid calculation and geohash generation for GeoJSON types.

use super::{
    ops::{encode_point, is_valid_coordinate},
    INDEX_PRECISIONS,
};
use raisin_models::nodes::properties::GeoJson;

/// Extract centroid coordinates from a GeoJSON geometry
///
/// For complex geometries (polygons, linestrings), calculates the centroid
/// for geohash indexing purposes.
pub fn geometry_centroid(geojson: &GeoJson) -> Option<(f64, f64)> {
    let centroid = match geojson {
        GeoJson::Point { coordinates } => Some((coordinates[0], coordinates[1])),
        GeoJson::LineString { coordinates } => {
            if coordinates.is_empty() {
                return None;
            }
            let count = coordinates.len() as f64;
            let sum_lon: f64 = coordinates.iter().map(|c| c[0]).sum();
            let sum_lat: f64 = coordinates.iter().map(|c| c[1]).sum();
            Some((sum_lon / count, sum_lat / count))
        }
        GeoJson::Polygon { coordinates } => {
            let exterior = coordinates.first()?;
            if exterior.is_empty() {
                return None;
            }
            let count = exterior.len() as f64;
            let sum_lon: f64 = exterior.iter().map(|c| c[0]).sum();
            let sum_lat: f64 = exterior.iter().map(|c| c[1]).sum();
            Some((sum_lon / count, sum_lat / count))
        }
        GeoJson::MultiPoint { coordinates } => {
            if coordinates.is_empty() {
                return None;
            }
            let count = coordinates.len() as f64;
            let sum_lon: f64 = coordinates.iter().map(|c| c[0]).sum();
            let sum_lat: f64 = coordinates.iter().map(|c| c[1]).sum();
            Some((sum_lon / count, sum_lat / count))
        }
        GeoJson::MultiLineString { coordinates } => {
            let all_points: Vec<_> = coordinates.iter().flatten().collect();
            if all_points.is_empty() {
                return None;
            }
            let count = all_points.len() as f64;
            let sum_lon: f64 = all_points.iter().map(|c| c[0]).sum();
            let sum_lat: f64 = all_points.iter().map(|c| c[1]).sum();
            Some((sum_lon / count, sum_lat / count))
        }
        GeoJson::MultiPolygon { coordinates } => {
            let all_points: Vec<_> = coordinates
                .iter()
                .filter_map(|poly| poly.first())
                .flatten()
                .collect();
            if all_points.is_empty() {
                return None;
            }
            let count = all_points.len() as f64;
            let sum_lon: f64 = all_points.iter().map(|c| c[0]).sum();
            let sum_lat: f64 = all_points.iter().map(|c| c[1]).sum();
            Some((sum_lon / count, sum_lat / count))
        }
        GeoJson::GeometryCollection { geometries } => {
            let centroids: Vec<_> = geometries.iter().filter_map(geometry_centroid).collect();
            if centroids.is_empty() {
                return None;
            }
            let count = centroids.len() as f64;
            let sum_lon: f64 = centroids.iter().map(|(lon, _)| lon).sum();
            let sum_lat: f64 = centroids.iter().map(|(_, lat)| lat).sum();
            Some((sum_lon / count, sum_lat / count))
        }
    };
    centroid.filter(|&(lon, lat)| {
        let valid = is_valid_coordinate(lon, lat);
        if !valid {
            tracing::warn!(lon, lat, "Computed geometry centroid has invalid coordinates");
        }
        valid
    })
}

/// Generate all geohashes for indexing a geometry
///
/// Returns multi-precision geohashes for the geometry's centroid.
pub fn geohashes_for_geometry(geojson: &GeoJson) -> Vec<String> {
    geometry_centroid(geojson)
        .map(|(lon, lat)| {
            INDEX_PRECISIONS
                .iter()
                .filter_map(|&p| encode_point(lon, lat, p))
                .collect()
        })
        .unwrap_or_default()
}
