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

//! Round-trip coverage for the model <-> geo direction.
//!
//! Every geometry type, not a representative sample: "no converter for Multi\*"
//! was the root cause of `Multi*` being unsupported across all 49 ST_\*
//! functions, so the fix is only proven by enumerating them.

use super::*;
use geo::{CoordsIter, Rect, Triangle};

fn p(x: f64, y: f64) -> Position {
    Position::new_2d(x, y)
}

/// One instance of each of the seven GeoJSON types, plus the degenerate shapes.
fn all_types() -> Vec<GeoJson> {
    vec![
        GeoJson::point(1.0, 2.0),
        GeoJson::LineString {
            coordinates: vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0)],
            srid: None,
        },
        // Exterior ring plus a hole — the interior-ring case.
        GeoJson::Polygon {
            coordinates: vec![
                vec![
                    p(0.0, 0.0),
                    p(10.0, 0.0),
                    p(10.0, 10.0),
                    p(0.0, 10.0),
                    p(0.0, 0.0),
                ],
                vec![
                    p(2.0, 2.0),
                    p(4.0, 2.0),
                    p(4.0, 4.0),
                    p(2.0, 4.0),
                    p(2.0, 2.0),
                ],
            ],
            srid: None,
        },
        GeoJson::MultiPoint {
            coordinates: vec![p(0.0, 0.0), p(1.0, 1.0)],
            srid: None,
        },
        GeoJson::MultiLineString {
            coordinates: vec![
                vec![p(0.0, 0.0), p(1.0, 1.0)],
                vec![p(2.0, 2.0), p(3.0, 3.0)],
            ],
            srid: None,
        },
        GeoJson::MultiPolygon {
            coordinates: vec![
                vec![vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 0.0)]],
                vec![
                    vec![p(5.0, 5.0), p(7.0, 5.0), p(7.0, 7.0), p(5.0, 5.0)],
                    vec![p(5.5, 5.5), p(6.0, 5.5), p(6.0, 6.0), p(5.5, 5.5)],
                ],
            ],
            srid: None,
        },
        GeoJson::GeometryCollection {
            geometries: vec![
                GeoJson::point(1.0, 1.0),
                GeoJson::LineString {
                    coordinates: vec![p(0.0, 0.0), p(1.0, 1.0)],
                    srid: None,
                },
                // Nesting: a collection inside a collection.
                GeoJson::GeometryCollection {
                    geometries: vec![GeoJson::point(9.0, 9.0)],
                    srid: None,
                },
            ],
            srid: None,
        },
    ]
}

#[test]
fn every_geometry_type_round_trips_through_geo() {
    for g in all_types() {
        let geom = to_geo_from_model(&g, None)
            .unwrap_or_else(|e| panic!("{} failed to convert: {e}", g.geometry_type()));
        let back = to_model(&geom).unwrap();
        assert_eq!(back, g, "{} did not round trip", g.geometry_type());
    }
}

#[test]
fn geo_variant_matches_the_geojson_type() {
    let expected = [
        ("Point", "Point"),
        ("LineString", "LineString"),
        ("Polygon", "Polygon"),
        ("MultiPoint", "MultiPoint"),
        ("MultiLineString", "MultiLineString"),
        ("MultiPolygon", "MultiPolygon"),
        ("GeometryCollection", "GeometryCollection"),
    ];
    for (g, (name, _)) in all_types().iter().zip(expected) {
        assert_eq!(g.geometry_type(), name);
        let geom = to_geo_from_model(g, None).unwrap();
        // Cheap structural check that we did not collapse a type.
        assert_eq!(
            to_model(&geom).unwrap().geometry_type(),
            name,
            "type changed for {name}"
        );
    }
}

#[test]
fn interior_rings_survive_in_order() {
    let g = &all_types()[2];
    let geom = to_geo_from_model(g, None).unwrap();
    match &geom.geometry {
        Geometry::Polygon(poly) => {
            assert_eq!(poly.interiors().len(), 1);
            assert_eq!(poly.exterior().coords_count(), 5);
            assert_eq!(poly.interiors()[0].coords_count(), 5);
        }
        other => panic!("expected Polygon, got {other:?}"),
    }
}

#[test]
fn empty_and_degenerate_shapes_convert_rather_than_erroring() {
    let cases = vec![
        GeoJson::empty(),
        GeoJson::LineString {
            coordinates: vec![],
            srid: None,
        },
        GeoJson::Polygon {
            coordinates: vec![],
            srid: None,
        },
        GeoJson::MultiPoint {
            coordinates: vec![],
            srid: None,
        },
        GeoJson::MultiPolygon {
            coordinates: vec![],
            srid: None,
        },
        // A single-vertex "line" and a two-vertex "polygon": legal GeoJSON
        // shapes that are topologically degenerate. They must convert; validity
        // is ST_ISVALID's business, not the converter's.
        GeoJson::LineString {
            coordinates: vec![p(1.0, 1.0)],
            srid: None,
        },
        GeoJson::Polygon {
            coordinates: vec![vec![p(0.0, 0.0), p(1.0, 1.0)]],
            srid: None,
        },
    ];
    for g in cases {
        let geom = to_geo_from_model(&g, None)
            .unwrap_or_else(|e| panic!("{} failed: {e}", g.geometry_type()));
        // Emptiness propagates rather than becoming an error.
        assert_eq!(geom.is_empty(), g.is_empty(), "{}", g.geometry_type());
        to_model(&geom).unwrap();
    }
}

#[test]
fn non_finite_coordinates_are_rejected_not_propagated() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let g = GeoJson::Point {
            coordinates: Position::new_2d(bad, 0.0),
            srid: None,
        };
        assert!(
            matches!(
                to_geo_from_model(&g, None),
                Err(GeometryError::NonFiniteCoordinate { .. })
            ),
            "{bad} must be rejected"
        );
    }
    // ...and from deep inside a nested geometry, not just the top level.
    let g = GeoJson::MultiPolygon {
        coordinates: vec![vec![vec![p(0.0, 0.0), Position::new_2d(1.0, f64::NAN)]]],
        srid: None,
    };
    assert!(to_geo_from_model(&g, None).is_err());
}

#[test]
fn altitude_is_read_into_z_range_and_dropped_from_the_coordinates() {
    let g = GeoJson::LineString {
        coordinates: vec![
            Position::new_3d(0.0, 0.0, 5.0),
            Position::new_3d(1.0, 1.0, 15.0),
        ],
        srid: None,
    };
    let geom = to_geo_from_model(&g, None).unwrap();
    assert_eq!(geom.z_range, Some((5.0, 15.0)));

    // geo is strictly 2-D, so the model coming back is 2-D. This is the
    // documented one-way loss, not a bug.
    let back = to_model(&geom).unwrap();
    assert_eq!(back.z_range(), None);
}

#[test]
fn srid_precedence_is_member_then_schema_then_wgs84() {
    let unlabelled = GeoJson::point(8.54, 47.37);
    assert_eq!(
        to_geo_from_model(&unlabelled, None).unwrap().srid,
        Crs::WGS84
    );
    assert_eq!(
        to_geo_from_model(&unlabelled, Some(2056)).unwrap().srid,
        Crs::from_srid(2056)
    );

    let labelled = unlabelled.clone().with_srid(Some(3857));
    assert_eq!(
        to_geo_from_model(&labelled, Some(2056)).unwrap().srid,
        Crs::WEB_MERCATOR,
        "an explicit member beats the schema default"
    );

    // A deprecated alias normalizes, so 900913 and 3857 are the same CRS and
    // never look like a mismatch to each other.
    let alias = unlabelled.with_srid(Some(900_913));
    assert_eq!(
        to_geo_from_model(&alias, None).unwrap().srid,
        Crs::WEB_MERCATOR
    );
}

#[test]
fn to_model_emits_srid_only_when_it_is_not_wgs84() {
    let geom = Geom::wgs84(Geometry::Point(geo::Point::new(1.0, 2.0)));
    assert_eq!(to_model(&geom).unwrap().srid(), None);

    let projected = Geom::new(
        Geometry::Point(geo::Point::new(1.0, 2.0)),
        Crs::from_srid(32632),
    );
    assert_eq!(to_model(&projected).unwrap().srid(), Some(32632));
}

/// `geo` has three geometry variants GeoJSON lacks, and its own algorithms
/// produce them — `BoundingRect` returns a `Rect`. Widening them is what makes
/// `ST_ENVELOPE` expressible at all.
#[test]
fn geo_only_variants_widen_instead_of_failing() {
    let rect = Geom::wgs84(Geometry::Rect(Rect::new(
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 2.0, y: 3.0 },
    )));
    let model = to_model(&rect).unwrap();
    assert_eq!(model.geometry_type(), "Polygon");
    // A closed ring: 4 corners plus the repeated first vertex.
    match model {
        GeoJson::Polygon { coordinates, .. } => assert_eq!(coordinates[0].len(), 5),
        other => panic!("expected Polygon, got {other:?}"),
    }

    let tri = Geom::wgs84(Geometry::Triangle(Triangle::new(
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 0.0 },
        Coord { x: 0.0, y: 1.0 },
    )));
    assert_eq!(to_model(&tri).unwrap().geometry_type(), "Polygon");

    let line = Geom::wgs84(Geometry::Line(geo::Line::new(
        Coord { x: 0.0, y: 0.0 },
        Coord { x: 1.0, y: 1.0 },
    )));
    let model = to_model(&line).unwrap();
    assert_eq!(model.geometry_type(), "LineString");
    match model {
        GeoJson::LineString { coordinates, .. } => assert_eq!(coordinates.len(), 2),
        other => panic!("expected LineString, got {other:?}"),
    }
}
