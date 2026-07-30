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

//! Splitting a geometry into its 2-D, 1-D and 0-D parts, and putting the result
//! back together as the narrowest type that represents it.
//!
//! This is what makes the set operations total. `geo`'s `BooleanOps` is
//! implemented for `Polygon` and `MultiPolygon` **only** — there is no impl for
//! `Point`, `LineString` or `Geometry` — so a set operation over mixed input
//! cannot simply delegate. Decomposing by dimension lets each dimension be
//! handled by the right algorithm and the pieces recombined, which is how
//! `ST_UNION(point, polygon)` becomes a defined answer instead of an
//! "unsupported geometry type" error.

use geo::{Geometry, MultiLineString, MultiPoint, MultiPolygon, Point};

/// A geometry separated by topological dimension.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Parts {
    pub polygons: MultiPolygon<f64>,
    pub lines: MultiLineString<f64>,
    pub points: MultiPoint<f64>,
}

// `Default` is written out rather than derived: `geo`'s `Multi*` types do not
// implement it, since an empty MultiPolygon is a meaningful geometry rather than
// an obvious zero value.
impl Default for Parts {
    fn default() -> Self {
        Parts {
            polygons: MultiPolygon(Vec::new()),
            lines: MultiLineString(Vec::new()),
            points: MultiPoint(Vec::new()),
        }
    }
}

/// Split a geometry by dimension, flattening nested collections.
///
/// `Rect` and `Triangle` become polygons and `Line` becomes a two-point
/// `LineString`: they are `geo`-only types with no GeoJSON spelling, but `geo`'s
/// own algorithms produce them (`BoundingRect` returns a `Rect`), so dropping
/// them would lose data.
pub(super) fn decompose(g: &Geometry<f64>) -> Parts {
    let mut parts = Parts::default();
    collect(g, &mut parts);
    parts
}

fn collect(g: &Geometry<f64>, out: &mut Parts) {
    match g {
        Geometry::Point(p) => out.points.0.push(*p),
        Geometry::MultiPoint(mp) => out.points.0.extend(mp.0.iter().copied()),
        Geometry::Line(l) => out
            .lines
            .0
            .push(geo::LineString::from(vec![l.start, l.end])),
        Geometry::LineString(ls) => out.lines.0.push(ls.clone()),
        Geometry::MultiLineString(mls) => out.lines.0.extend(mls.0.iter().cloned()),
        Geometry::Polygon(p) => out.polygons.0.push(p.clone()),
        Geometry::MultiPolygon(mp) => out.polygons.0.extend(mp.0.iter().cloned()),
        Geometry::Rect(r) => out.polygons.0.push(r.to_polygon()),
        Geometry::Triangle(t) => out.polygons.0.push(t.to_polygon()),
        Geometry::GeometryCollection(gc) => gc.0.iter().for_each(|m| collect(m, out)),
    }
}

/// Rebuild a geometry from its parts, choosing the narrowest representation.
///
/// A single polygon is a `Polygon`, not a one-element `MultiPolygon`; an empty
/// result is the canonical empty `GeometryCollection`; and a result spanning more
/// than one dimension is a `GeometryCollection` of one homogeneous member per
/// dimension, ordered high dimension first (as PostGIS does).
pub(super) fn recompose(mut parts: Parts) -> Geometry<f64> {
    parts
        .points
        .0
        .retain(|p| p.x().is_finite() && p.y().is_finite());
    parts.lines.0.retain(|ls| ls.0.len() >= 2);
    dedup_points(&mut parts.points);

    let mut members: Vec<Geometry<f64>> = Vec::with_capacity(3);
    if !parts.polygons.0.is_empty() {
        members.push(narrowest(parts.polygons.0, Geometry::Polygon, |v| {
            Geometry::MultiPolygon(MultiPolygon(v))
        }));
    }
    if !parts.lines.0.is_empty() {
        members.push(narrowest(parts.lines.0, Geometry::LineString, |v| {
            Geometry::MultiLineString(MultiLineString(v))
        }));
    }
    if !parts.points.0.is_empty() {
        members.push(narrowest(parts.points.0, Geometry::Point, |v| {
            Geometry::MultiPoint(MultiPoint(v))
        }));
    }

    match members.len() {
        0 => Geometry::GeometryCollection(Default::default()),
        1 => members.remove(0),
        _ => Geometry::GeometryCollection(members.into()),
    }
}

fn narrowest<T>(
    mut items: Vec<T>,
    single: impl FnOnce(T) -> Geometry<f64>,
    multi: impl FnOnce(Vec<T>) -> Geometry<f64>,
) -> Geometry<f64> {
    if items.len() == 1 {
        single(items.remove(0))
    } else {
        multi(items)
    }
}

/// Drop repeated coordinates, which a union of point sets otherwise duplicates.
///
/// Compared on raw bits rather than with a tolerance: a set operation must be
/// idempotent, and an epsilon would make the output depend on insertion order.
fn dedup_points(points: &mut MultiPoint<f64>) {
    let mut seen: Vec<(u64, u64)> = Vec::with_capacity(points.0.len());
    points.0.retain(|p: &Point<f64>| {
        let key = (p.x().to_bits(), p.y().to_bits());
        if seen.contains(&key) {
            false
        } else {
            seen.push(key);
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{LineString, Polygon};

    fn square() -> Polygon<f64> {
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

    #[test]
    fn a_nested_collection_flattens_into_three_dimensions() {
        let g = Geometry::GeometryCollection(
            vec![
                Geometry::GeometryCollection(
                    vec![
                        Geometry::Point(Point::new(1.0, 1.0)),
                        Geometry::Polygon(square()),
                    ]
                    .into(),
                ),
                Geometry::LineString(LineString::from(vec![(0.0, 0.0), (2.0, 2.0)])),
            ]
            .into(),
        );
        let p = decompose(&g);
        assert_eq!(p.polygons.0.len(), 1);
        assert_eq!(p.lines.0.len(), 1);
        assert_eq!(p.points.0.len(), 1);
    }

    #[test]
    fn recompose_picks_the_minimal_type() {
        assert!(matches!(
            recompose(Parts::default()),
            Geometry::GeometryCollection(_)
        ));

        let one_poly = Parts {
            polygons: MultiPolygon(vec![square()]),
            ..Default::default()
        };
        assert!(matches!(recompose(one_poly), Geometry::Polygon(_)));

        let two_poly = Parts {
            polygons: MultiPolygon(vec![square(), square()]),
            ..Default::default()
        };
        assert!(matches!(recompose(two_poly), Geometry::MultiPolygon(_)));
    }

    #[test]
    fn a_mixed_result_becomes_a_collection_ordered_by_dimension() {
        let mixed = Parts {
            polygons: MultiPolygon(vec![square()]),
            lines: MultiLineString(vec![LineString::from(vec![(5.0, 5.0), (6.0, 6.0)])]),
            points: MultiPoint(vec![Point::new(9.0, 9.0)]),
        };
        match recompose(mixed) {
            Geometry::GeometryCollection(gc) => {
                assert_eq!(gc.0.len(), 3);
                assert!(matches!(gc.0[0], Geometry::Polygon(_)));
                assert!(matches!(gc.0[1], Geometry::LineString(_)));
                assert!(matches!(gc.0[2], Geometry::Point(_)));
            }
            other => panic!("expected a collection, got {other:?}"),
        }
    }

    #[test]
    fn duplicate_points_and_degenerate_lines_are_dropped() {
        let parts = Parts {
            points: MultiPoint(vec![
                Point::new(1.0, 1.0),
                Point::new(1.0, 1.0),
                Point::new(2.0, 2.0),
            ]),
            lines: MultiLineString(vec![
                LineString::from(vec![(0.0, 0.0)]),
                LineString::from(vec![(0.0, 0.0), (1.0, 1.0)]),
            ]),
            ..Default::default()
        };
        match recompose(parts) {
            Geometry::GeometryCollection(gc) => {
                assert!(
                    matches!(gc.0[0], Geometry::LineString(_)),
                    "one line survives"
                );
                match &gc.0[1] {
                    Geometry::MultiPoint(mp) => assert_eq!(mp.0.len(), 2, "duplicate dropped"),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
    }
}
