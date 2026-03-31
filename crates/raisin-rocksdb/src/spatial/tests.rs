//! Tests for spatial indexing utilities.

use super::*;
use raisin_models::nodes::properties::GeoJson;

#[test]
fn test_encode_decode_roundtrip() {
    let lon = -122.4194; // San Francisco
    let lat = 37.7749;

    let hash = encode_point(lon, lat, 8);
    let (decoded_lon, decoded_lat) = decode_geohash(&hash).unwrap();

    assert!((decoded_lon - lon).abs() < 0.001);
    assert!((decoded_lat - lat).abs() < 0.001);
}

#[test]
fn test_neighbors() {
    let hash = encode_point(-122.4194, 37.7749, 6);
    let neighbor_hashes = neighbors(&hash);

    assert_eq!(neighbor_hashes.len(), 8);

    for n in &neighbor_hashes {
        assert_eq!(n.len(), hash.len());
    }
}

#[test]
fn test_center_and_neighbors() {
    let hash = "9q8yyk";
    let cells = center_and_neighbors(hash);

    assert_eq!(cells.len(), 9);
    assert_eq!(cells[0], hash);
}

#[test]
fn test_multi_precision_geohashes() {
    let hashes = multi_precision_geohashes(-122.4194, 37.7749);

    assert_eq!(hashes.len(), 5);
    assert_eq!(hashes[0].0, 4);
    assert_eq!(hashes[4].0, 8);
    assert!(hashes[0].1.len() < hashes[4].1.len());
}

#[test]
fn test_geometry_centroid_point() {
    let point = GeoJson::Point {
        coordinates: [-122.4194, 37.7749],
    };

    let (lon, lat) = geometry_centroid(&point).unwrap();
    assert!((lon - (-122.4194)).abs() < 0.0001);
    assert!((lat - 37.7749).abs() < 0.0001);
}

#[test]
fn test_geometry_centroid_polygon() {
    let polygon = GeoJson::Polygon {
        coordinates: vec![vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            [0.0, 0.0],
        ]],
    };

    let (lon, lat) = geometry_centroid(&polygon).unwrap();
    assert!((lon - 0.4).abs() < 0.1);
    assert!((lat - 0.4).abs() < 0.1);
}

#[test]
fn test_precision_for_radius() {
    let precision_100m = precision_for_radius(100.0);
    assert!(precision_100m >= 7);
    assert!(precision_100m <= 8);

    let precision_1km = precision_for_radius(1000.0);
    assert!(precision_1km >= 5);
    assert!(precision_1km <= 6);

    let precision_10km = precision_for_radius(10000.0);
    assert!(precision_10km >= 4);
    assert!(precision_10km <= 5);
}

#[test]
fn test_cells_for_radius() {
    let cells = cells_for_radius(-122.4194, 37.7749, 500.0);

    assert_eq!(cells.len(), 9);

    let first_len = cells[0].len();
    for cell in &cells {
        assert_eq!(cell.len(), first_len);
    }
}

#[test]
fn test_geohashes_for_geometry() {
    let point = GeoJson::Point {
        coordinates: [-122.4194, 37.7749],
    };

    let hashes = geohashes_for_geometry(&point);
    assert_eq!(hashes.len(), 5);
}
