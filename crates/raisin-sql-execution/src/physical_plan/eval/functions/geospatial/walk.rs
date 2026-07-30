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

//! Recursive traversal of a `geo::Geometry` by dimension.
//!
//! `geo` has no single "give me every linear component" accessor, and the
//! alternative — a `match` over eleven variants inside every measurement
//! function — is precisely how the old implementation ended up supporting a
//! different, incomplete set of types in each of forty-nine places. These four
//! walkers exist so that `Multi*` and `GeometryCollection` (including nested
//! collections) are handled once.
//!
//! `Line`, `Rect` and `Triangle` are `geo`-only types with no GeoJSON spelling,
//! but they arrive from `geo`'s own algorithms (`BoundingRect` returns a `Rect`),
//! so every walker handles them rather than ignoring them.

use geo::{Geometry, LineString, Point, Polygon};

/// Visit every 0-dimensional component.
pub(super) fn for_each_point(g: &Geometry<f64>, f: &mut impl FnMut(Point<f64>)) {
    match g {
        Geometry::Point(p) => f(*p),
        Geometry::MultiPoint(mp) => mp.0.iter().for_each(|p| f(*p)),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|m| for_each_point(m, f)),
        _ => {}
    }
}

/// Visit every 1-dimensional component, as a `LineString`.
///
/// Polygon rings are **not** included: they bound an area rather than being
/// linear components, which is why `ST_LENGTH` of a polygon is 0 and
/// `ST_PERIMETER` exists separately.
pub(super) fn for_each_line_string(g: &Geometry<f64>, f: &mut impl FnMut(&LineString<f64>)) {
    match g {
        Geometry::LineString(ls) => f(ls),
        Geometry::MultiLineString(mls) => mls.0.iter().for_each(f),
        Geometry::Line(l) => f(&LineString::from(vec![l.start, l.end])),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|m| for_each_line_string(m, f)),
        _ => {}
    }
}

/// Visit every 2-dimensional component, as a `Polygon`.
pub(super) fn for_each_polygon(g: &Geometry<f64>, f: &mut impl FnMut(&Polygon<f64>)) {
    match g {
        Geometry::Polygon(p) => f(p),
        Geometry::MultiPolygon(mp) => mp.0.iter().for_each(f),
        Geometry::Rect(r) => f(&r.to_polygon()),
        Geometry::Triangle(t) => f(&t.to_polygon()),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|m| for_each_polygon(m, f)),
        _ => {}
    }
}

/// Visit every ring of every 2-dimensional component, exterior and interior
/// alike.
pub(super) fn for_each_ring(g: &Geometry<f64>, f: &mut impl FnMut(&LineString<f64>)) {
    for_each_polygon(g, &mut |p| {
        f(p.exterior());
        p.interiors().iter().for_each(&mut *f);
    });
}

/// The single `Point` a geometry consists of, if it consists of exactly one.
///
/// Used to take the exact-geodesic fast path in `ST_DISTANCE` / `ST_AZIMUTH`: a
/// `MultiPoint` of one and a `Point` are the same location, and treating them
/// differently would make the answer depend on the spelling.
pub(super) fn single_point(g: &Geometry<f64>) -> Option<Point<f64>> {
    let mut found = None;
    let mut count = 0usize;
    for_each_point(g, &mut |p| {
        count += 1;
        if count == 1 {
            found = Some(p);
        }
    });
    // Any linear or areal component disqualifies the fast path.
    let mut has_other = false;
    for_each_line_string(g, &mut |_| has_other = true);
    for_each_polygon(g, &mut |_| has_other = true);
    (count == 1 && !has_other).then_some(found).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Coord, Line, MultiPoint, MultiPolygon, Rect};

    fn ring() -> Polygon<f64> {
        Polygon::new(
            LineString::from(vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]),
            vec![LineString::from(vec![
                (0.2, 0.2),
                (0.4, 0.2),
                (0.3, 0.4),
                (0.2, 0.2),
            ])],
        )
    }

    #[test]
    fn nested_collections_are_traversed_to_the_bottom() {
        let inner = Geometry::GeometryCollection(
            vec![
                Geometry::Point(Point::new(1.0, 1.0)),
                Geometry::Polygon(ring()),
            ]
            .into(),
        );
        let outer = Geometry::GeometryCollection(
            vec![
                inner,
                Geometry::LineString(LineString::from(vec![(0.0, 0.0), (1.0, 1.0)])),
            ]
            .into(),
        );

        let mut points = 0;
        for_each_point(&outer, &mut |_| points += 1);
        let mut lines = 0;
        for_each_line_string(&outer, &mut |_| lines += 1);
        let mut polys = 0;
        for_each_polygon(&outer, &mut |_| polys += 1);
        let mut rings = 0;
        for_each_ring(&outer, &mut |_| rings += 1);

        assert_eq!((points, lines, polys, rings), (1, 1, 1, 2));
    }

    /// A polygon's rings must not be reported as linear components, or
    /// `ST_LENGTH` of a polygon becomes its perimeter — which is not what PostGIS
    /// returns.
    #[test]
    fn polygon_rings_are_not_linear_components() {
        let g = Geometry::Polygon(ring());
        let mut lines = 0;
        for_each_line_string(&g, &mut |_| lines += 1);
        assert_eq!(lines, 0);
    }

    #[test]
    fn geo_only_types_are_handled_rather_than_skipped() {
        let mut lines = 0;
        for_each_line_string(
            &Geometry::Line(Line::new(
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 3.0, y: 4.0 },
            )),
            &mut |ls| {
                lines += 1;
                assert_eq!(ls.0.len(), 2);
            },
        );
        assert_eq!(lines, 1);

        let mut polys = 0;
        for_each_polygon(
            &Geometry::Rect(Rect::new(
                Coord { x: 0.0, y: 0.0 },
                Coord { x: 1.0, y: 1.0 },
            )),
            &mut |_| polys += 1,
        );
        assert_eq!(polys, 1);
    }

    #[test]
    fn single_point_ignores_spelling_but_not_extra_content() {
        assert!(single_point(&Geometry::Point(Point::new(1.0, 2.0))).is_some());
        assert!(
            single_point(&Geometry::MultiPoint(MultiPoint(vec![Point::new(
                1.0, 2.0
            )])))
            .is_some(),
            "a MultiPoint of one is the same location as a Point"
        );
        assert!(single_point(&Geometry::MultiPoint(MultiPoint(vec![
            Point::new(1.0, 2.0),
            Point::new(3.0, 4.0)
        ])))
        .is_none());
        assert!(single_point(&Geometry::MultiPolygon(MultiPolygon(vec![ring()]))).is_none());
    }
}
