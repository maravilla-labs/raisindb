//! Unit tests for [`super::normalize_geometry_for_index`].
//!
//! Split out of `normalize.rs` to keep both files under the 300-line limit.

use super::*;

/// Zurich, in WGS84 and the same place in EPSG:3857.
const ZURICH_LON: f64 = 8.5417;
const ZURICH_LAT: f64 = 47.3769;

fn mercator(lon: f64, lat: f64) -> (f64, f64) {
    raisin_proj::transform_coord(Crs::WGS84, Crs::WEB_MERCATOR, lon, lat).unwrap()
}

#[test]
fn unlabelled_and_4326_are_borrowed_untouched() {
    let g = GeoJson::point(ZURICH_LON, ZURICH_LAT);
    assert!(matches!(
        normalize_geometry_for_index(&g).unwrap(),
        Cow::Borrowed(_)
    ));

    let labelled = GeoJson::point(ZURICH_LON, ZURICH_LAT).with_srid(Some(4326));
    assert!(matches!(
        normalize_geometry_for_index(&labelled).unwrap(),
        Cow::Borrowed(_)
    ));
}

#[test]
fn web_mercator_point_round_trips_to_the_original_degrees() {
    let (x, y) = mercator(ZURICH_LON, ZURICH_LAT);
    let g = GeoJson::point(x, y).with_srid(Some(3857));

    let normalized = normalize_geometry_for_index(&g).unwrap();
    let p = normalized.as_point().expect("still a point");
    assert!((p.x - ZURICH_LON).abs() < 1e-9, "{p:?}");
    assert!((p.y - ZURICH_LAT).abs() < 1e-9, "{p:?}");
    // The normalised copy is WGS84 and says so by carrying no label.
    assert_eq!(normalized.srid(), None);
    // The ORIGINAL is untouched: normalisation is a read, not a rewrite.
    assert_eq!(g.srid(), Some(3857));
    assert_eq!(g.as_point().unwrap().x, x);
}

#[test]
fn the_web_mercator_aliases_normalize_identically() {
    let (x, y) = mercator(ZURICH_LON, ZURICH_LAT);
    let a = normalize_geometry_for_index(&GeoJson::point(x, y).with_srid(Some(3857)))
        .unwrap()
        .into_owned();
    for alias in [3785, 900_913] {
        let b = normalize_geometry_for_index(&GeoJson::point(x, y).with_srid(Some(alias)))
            .unwrap()
            .into_owned();
        assert_eq!(a, b, "alias {alias} must produce identical index geometry");
    }
}

#[test]
fn altitude_survives_a_horizontal_reprojection() {
    let (x, y) = mercator(ZURICH_LON, ZURICH_LAT);
    let g = GeoJson::point_3d(x, y, 408.0).with_srid(Some(3857));
    let normalized = normalize_geometry_for_index(&g).unwrap();
    assert_eq!(normalized.as_point().unwrap().z, Some(408.0));
    assert_eq!(normalized.z_range(), Some((408.0, 408.0)));
}

#[test]
fn a_utm_polygon_normalizes_every_vertex_and_keeps_its_shape() {
    let corners = [
        (8.50, 47.35),
        (8.60, 47.35),
        (8.60, 47.40),
        (8.50, 47.40),
        (8.50, 47.35),
    ];
    let utm = Crs::from_srid(32632);
    let ring: Vec<Position> = corners
        .iter()
        .map(|&(lon, lat)| {
            let (x, y) = raisin_proj::transform_coord(Crs::WGS84, utm, lon, lat).unwrap();
            Position::new_2d(x, y)
        })
        .collect();
    let g = GeoJson::Polygon {
        coordinates: vec![ring],
        srid: Some(32632),
    };

    let normalized = normalize_geometry_for_index(&g).unwrap();
    let GeoJson::Polygon { coordinates, .. } = normalized.as_ref() else {
        panic!("still a polygon");
    };
    assert_eq!(coordinates[0].len(), corners.len(), "no vertex dropped");
    for (got, &(lon, lat)) in coordinates[0].iter().zip(corners.iter()) {
        assert!((got.x - lon).abs() < 1e-6, "{got:?} vs {lon}");
        assert!((got.y - lat).abs() < 1e-6, "{got:?} vs {lat}");
    }
}

#[test]
fn an_unsupported_srid_is_a_loud_validation_error_naming_the_srid() {
    // Belgian Lambert 72: a real CRS that tier 2 and tier 3 know and the
    // built-in tier does not.
    let g = GeoJson::point(150_000.0, 170_000.0).with_srid(Some(31_370));
    let err = normalize_geometry_for_index(&g).unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, Error::Validation(_)), "{err:?}");
    assert!(msg.contains("31370"), "must name the SRID: {msg}");
    assert!(
        msg.contains("ST_TRANSFORM"),
        "must say how to fix it: {msg}"
    );
    assert!(
        msg.contains("proj-backend"),
        "must name the Cargo features and that they do not help: {msg}"
    );
}

/// The cluster-determinism guarantee, restated at the layer that actually
/// writes bytes. Enabling `proj4rs-backend` / `proj-backend` must not change
/// which geometries get indexed, or two nodes in one cluster would hold
/// different index contents for the same replicated record.
#[test]
fn feature_flags_never_widen_what_is_indexable() {
    for srid in [31_370, 2056, 27_700, 21_781] {
        assert!(
            !is_indexable_srid(srid),
            "EPSG:{srid} must be refused on every build configuration"
        );
        let g = GeoJson::point(1.0, 2.0).with_srid(Some(srid));
        assert!(normalize_geometry_for_index(&g).is_err(), "EPSG:{srid}");
    }
    for srid in [4326, 3857, 3785, 900_913, 32_632, 32_756] {
        assert!(
            is_indexable_srid(srid),
            "EPSG:{srid} must always be indexable"
        );
    }
}

#[test]
fn an_out_of_domain_position_is_dropped_not_fatal() {
    // A latitude beyond the WebMercator domain: the vertex disappears, the
    // rest of the line survives, and the write is not failed.
    let (ok_x, ok_y) = mercator(ZURICH_LON, ZURICH_LAT);
    let g = GeoJson::LineString {
        coordinates: vec![
            Position::new_2d(ok_x, ok_y),
            Position::new_2d(f64::NAN, ok_y),
        ],
        srid: Some(3857),
    };
    let normalized = normalize_geometry_for_index(&g).unwrap();
    let GeoJson::LineString { coordinates, .. } = normalized.as_ref() else {
        panic!("still a line");
    };
    assert_eq!(coordinates.len(), 1);
}
