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

//! `ST_ISVALID`, `ST_ISVALIDREASON` and `ST_MAKEVALID`, on `geo`'s OGC validation.
//!
//! The previous `ST_ISVALID` inspected the JSON's array shape — ring lengths and
//! whether the ordinates were numbers — so a self-intersecting bow-tie polygon
//! passed. `geo::Validation` implements the OGC Simple Feature rules for every
//! type, and `InvalidPolygon::SelfIntersection` is precisely the case that was
//! being missed.
//!
//! One documented limitation, inherited verbatim from `geo`: simple connectivity
//! of a polygon's interior is not checked, so rings that touch in a way that
//! splits the interior into two parts are reported valid.

use geo::{BooleanOps, Geometry, Validation};
use raisin_geometry::Geom;

use super::convert::narrow_multipolygon;
use super::setops::{self, SetOp};

/// True when the geometry satisfies the OGC validity rules.
pub(super) fn is_valid(g: &Geometry<f64>) -> bool {
    g.is_valid()
}

/// The first reason the geometry is invalid, in human-readable form.
///
/// `None` means valid. Only the first reason is reported, matching PostGIS's
/// `ST_IsValidReason`; `geo`'s `validation_errors()` would give them all, but a
/// single actionable sentence is what a user acts on.
pub(super) fn invalid_reason(g: &Geometry<f64>) -> Option<String> {
    g.check_validation().err().map(|e| e.to_string())
}

/// Repair an invalid geometry, preserving as much of it as possible.
///
/// The mechanism for areal geometry is a union of the polygonal parts with
/// themselves (`geo::BooleanOps`): an overlay recomputes the arrangement of the
/// edges from scratch, which is exactly what turns a self-intersecting bow-tie
/// into a valid two-polygon `MultiPolygon` and what merges overlapping rings.
///
/// Puntal and linear components cannot be invalid once they are here — a
/// non-finite ordinate is rejected at the conversion boundary, and a `LineString`
/// is allowed to self-intersect — so they pass through unchanged. That makes this
/// a no-op on already-valid input rather than a reshaping of it.
pub(super) fn make_valid(g: &Geom) -> Geom {
    if is_valid(&g.geometry) {
        return g.clone();
    }

    let repaired = match &g.geometry {
        Geometry::Polygon(p) => narrow_multipolygon(p.union(p)),
        Geometry::MultiPolygon(mp) => narrow_multipolygon(mp.union(mp)),
        Geometry::Rect(r) => narrow_multipolygon(r.to_polygon().union(&r.to_polygon())),
        Geometry::Triangle(t) => narrow_multipolygon(t.to_polygon().union(&t.to_polygon())),

        // A mixed geometry is repaired by unioning it with the canonical empty
        // geometry: the set-operation machinery decomposes by dimension, rebuilds
        // the polygonal arrangement, and reassembles the minimal type.
        other => {
            let empty = Geom::new(Geometry::GeometryCollection(Default::default()), g.srid);
            match setops::apply(SetOp::Union, &g.map_geometry(other.clone()), &empty) {
                Ok(fixed) => fixed.geometry,
                // A union cannot fail for two geometries in the same CRS, but
                // returning the input unchanged is the right answer if it ever
                // does: `ST_MAKEVALID` must never lose data.
                Err(_) => other.clone(),
            }
        }
    };

    g.map_geometry(repaired)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Area, LineString, MultiPolygon, Point, Polygon};

    fn bowtie() -> Polygon<f64> {
        Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (2.0, 2.0),
                (2.0, 0.0),
                (0.0, 2.0),
                (0.0, 0.0),
            ]),
            vec![],
        )
    }

    fn clean() -> Polygon<f64> {
        Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
                (0.0, 0.0),
            ]),
            vec![],
        )
    }

    /// The headline fix: the old array-shape check passed this polygon.
    #[test]
    fn a_self_intersecting_bowtie_is_invalid() {
        assert!(!is_valid(&Geometry::Polygon(bowtie())));
        let reason = invalid_reason(&Geometry::Polygon(bowtie())).expect("a reason");
        assert!(
            reason.contains("self-intersection"),
            "the reason must name the defect: {reason}"
        );
    }

    #[test]
    fn a_clean_polygon_is_valid_with_no_reason() {
        assert!(is_valid(&Geometry::Polygon(clean())));
        assert_eq!(invalid_reason(&Geometry::Polygon(clean())), None);
    }

    #[test]
    fn a_hole_outside_its_shell_is_invalid_and_says_so() {
        let bad = Polygon::new(
            clean().exterior().clone(),
            vec![LineString::from(vec![
                (5.0, 5.0),
                (6.0, 5.0),
                (6.0, 6.0),
                (5.0, 5.0),
            ])],
        );
        let reason = invalid_reason(&Geometry::Polygon(bad)).expect("a reason");
        assert!(reason.contains("exterior"), "{reason}");
    }

    #[test]
    fn a_ring_with_too_few_points_is_invalid() {
        let degenerate = Polygon::new(LineString::from(vec![(0.0, 0.0), (1.0, 1.0)]), vec![]);
        assert!(!is_valid(&Geometry::Polygon(degenerate)));
    }

    #[test]
    fn points_and_lines_are_valid_including_a_self_crossing_line() {
        assert!(is_valid(&Geometry::Point(Point::new(1.0, 2.0))));
        assert!(is_valid(&Geometry::LineString(LineString::from(vec![
            (0.0, 0.0),
            (2.0, 2.0),
            (2.0, 0.0),
            (0.0, 2.0)
        ]))));
    }

    #[test]
    fn validity_recurses_into_multi_and_collection_types() {
        assert!(!is_valid(&Geometry::MultiPolygon(MultiPolygon(vec![
            clean(),
            bowtie()
        ]))));
        assert!(!is_valid(&Geometry::GeometryCollection(
            vec![Geometry::Polygon(bowtie())].into()
        )));
        assert!(is_valid(&Geometry::GeometryCollection(Default::default())));
    }

    /// The repair must produce something valid AND keep the area.
    #[test]
    fn make_valid_turns_a_bowtie_into_two_valid_triangles() {
        let fixed = make_valid(&Geom::wgs84(Geometry::Polygon(bowtie())));
        assert!(is_valid(&fixed.geometry), "{:?}", fixed.geometry);
        assert!(
            matches!(fixed.geometry, Geometry::MultiPolygon(_)),
            "a bow-tie is genuinely two lobes: {:?}",
            fixed.geometry
        );
        // Each lobe of a 2x2 bow-tie is a triangle of area 1.
        assert!((fixed.geometry.unsigned_area() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn make_valid_leaves_valid_input_byte_identical() {
        let g = Geom::wgs84(Geometry::Polygon(clean()));
        assert_eq!(make_valid(&g), g);

        let p = Geom::wgs84(Geometry::Point(Point::new(1.0, 2.0)));
        assert_eq!(make_valid(&p), p);
    }

    #[test]
    fn make_valid_preserves_the_crs() {
        let utm = raisin_geometry::Crs::from_srid(32632);
        let g = Geom::new(Geometry::Polygon(bowtie()), utm);
        assert_eq!(make_valid(&g).srid, utm);
    }

    #[test]
    fn make_valid_repairs_an_invalid_member_of_a_collection() {
        let g = Geom::wgs84(Geometry::GeometryCollection(
            vec![Geometry::Polygon(bowtie())].into(),
        ));
        let fixed = make_valid(&g);
        assert!(is_valid(&fixed.geometry), "{:?}", fixed.geometry);
    }
}
