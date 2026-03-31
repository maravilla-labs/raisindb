//! Tests for the spatial index repository.

use super::repository::haversine_distance;

#[test]
fn test_haversine_distance() {
    // Distance from San Francisco to Los Angeles (~559 km)
    let sf_lon = -122.4194;
    let sf_lat = 37.7749;
    let la_lon = -118.2437;
    let la_lat = 34.0522;

    let distance = haversine_distance(sf_lon, sf_lat, la_lon, la_lat);

    // Should be approximately 559 km
    assert!((distance - 559_000.0).abs() < 10_000.0);
}

#[test]
fn test_haversine_same_point() {
    let distance = haversine_distance(-122.4194, 37.7749, -122.4194, 37.7749);
    assert!(distance < 0.001); // Should be essentially zero
}
