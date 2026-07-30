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

//! `ST_ISSIMPLE`: real self-intersection detection, replacing a constant `true`.
//!
//! # Why this is hand-written
//!
//! `geo` has no public simplicity API. Its own helper
//! `linestring_has_self_intersection` is `pub(crate)` and O(n²). The public tool
//! is `geo::sweep::Intersections`, a Bentley-Ottmann sweep that runs in
//! O(n log n) — but it reports intersecting *pairs* without saying which
//! vertices they came from, and simplicity is entirely a question of **which**
//! pairs are allowed to meet. So the segments are wrapped in a type that carries
//! its component and vertex index, which `geo::sweep::Cross` explicitly supports.
//!
//! # The rule
//!
//! A geometry is simple when the only intersections between its segments are the
//! shared vertices that consecutive segments must have:
//!
//! * a *proper* (interior) crossing is never allowed;
//! * a collinear overlap is never allowed, even between neighbours (that is a
//!   spike, where a line doubles back on itself);
//! * consecutive segments may meet at their shared vertex;
//! * a closed ring's first and last segments may meet at the closing vertex;
//! * distinct `MultiLineString` components may meet only at each other's
//!   boundary endpoints.
//!
//! Anything else — a figure-eight, a loop returning to an earlier vertex, a
//! repeated point in a `MultiPoint` — is not simple.

use geo::line_intersection::LineIntersection;
use geo::sweep::{Cross, Intersections};
use geo::{Coord, Geometry, Line, LineString, MultiLineString, Validation};

/// A segment tagged with where it came from, so the sweep's output can be judged.
#[derive(Debug, Clone, Copy)]
struct Seg {
    line: Line<f64>,
    /// Index of the owning `LineString` within the geometry.
    component: usize,
    /// Index of this segment within its component.
    index: usize,
}

impl Cross for Seg {
    type Scalar = f64;
    fn line(&self) -> Line<f64> {
        self.line
    }
}

/// True when the geometry has no anomalous self-intersection.
pub(super) fn is_simple(g: &Geometry<f64>) -> bool {
    match g {
        Geometry::Point(_) => true,
        Geometry::MultiPoint(mp) => {
            let mut seen: Vec<(u64, u64)> = Vec::with_capacity(mp.0.len());
            mp.0.iter().all(|p| {
                let key = (p.x().to_bits(), p.y().to_bits());
                if seen.contains(&key) {
                    false
                } else {
                    seen.push(key);
                    true
                }
            })
        }
        // A single segment cannot cross itself.
        Geometry::Line(_) => true,
        Geometry::LineString(ls) => linear_is_simple(&MultiLineString(vec![ls.clone()])),
        Geometry::MultiLineString(mls) => linear_is_simple(mls),

        // For areal geometry, simplicity is ring simplicity, which is exactly
        // what `geo`'s validation checks (`InvalidPolygon::SelfIntersection`).
        //
        // DIVERGENCE, deliberate: GEOS — and therefore PostGIS — returns `true`
        // for every polygon regardless of its rings. That is defensible under
        // OGC (validity is the polygon's concern) but it is indistinguishable
        // from the constant-`true` stub this module replaces, and it tells a user
        // with a bow-tie polygon nothing. We answer the useful question.
        Geometry::Polygon(_)
        | Geometry::MultiPolygon(_)
        | Geometry::Rect(_)
        | Geometry::Triangle(_) => g.is_valid(),

        Geometry::GeometryCollection(gc) => gc.0.iter().all(is_simple),
    }
}

fn linear_is_simple(mls: &MultiLineString<f64>) -> bool {
    let (segments, last_index) = segments_of(mls);
    if segments.len() < 2 {
        return true;
    }

    let closed: Vec<bool> = mls.0.iter().map(|ls| ls.is_closed()).collect();
    let boundaries: Vec<[Option<Coord<f64>>; 2]> = mls.0.iter().map(boundary_of).collect();

    for (a, b, kind) in Intersections::<Seg>::from_iter(segments) {
        let allowed = match kind {
            // A shared sub-segment is a spike or a doubling-back, never simple.
            LineIntersection::Collinear { .. } => false,
            LineIntersection::SinglePoint {
                is_proper: true, ..
            } => false,
            LineIntersection::SinglePoint { intersection, .. } => {
                if a.component == b.component {
                    let c = a.component;
                    // The sweep does not order the pair, so order it by vertex
                    // index: only then does "the shared vertex" have one
                    // spelling.
                    let (first, second) = if a.index <= b.index { (a, b) } else { (b, a) };

                    // Consecutive segments share the earlier one's end vertex.
                    let consecutive =
                        second.index == first.index + 1 && intersection == first.line.end;

                    // A closed ring's first and last segments share the closing
                    // vertex, which is where the ring starts.
                    let ring_seam = closed[c]
                        && first.index == 0
                        && second.index == last_index[c]
                        && intersection == first.line.start;

                    consecutive || ring_seam
                } else {
                    // Distinct components may only touch at each other's ends.
                    is_boundary(&boundaries[a.component], intersection)
                        && is_boundary(&boundaries[b.component], intersection)
                }
            }
        };
        if !allowed {
            return false;
        }
    }
    true
}

// Coordinates are compared exactly throughout. The sweep reports a genuinely
// shared vertex verbatim, so a tolerance would buy nothing and would hide real
// crossings that happen to land very close to a vertex.

/// The two boundary endpoints of a component, or `None` for a closed ring (which
/// has an empty boundary, so nothing may touch it).
fn boundary_of(ls: &LineString<f64>) -> [Option<Coord<f64>>; 2] {
    if ls.0.len() < 2 || ls.is_closed() {
        return [None, None];
    }
    [Some(ls.0[0]), Some(ls.0[ls.0.len() - 1])]
}

fn is_boundary(boundary: &[Option<Coord<f64>>; 2], c: Coord<f64>) -> bool {
    boundary.iter().flatten().any(|b| *b == c)
}

/// The non-degenerate segments of every component, plus the last segment index in
/// each component.
///
/// Zero-length segments — a repeated vertex — are dropped, because they intersect
/// their neighbour trivially and a repeated vertex is not an anomaly. The
/// **indices are then assigned over the surviving segments**, not over the original
/// ones. That renumbering is load-bearing: with the original indices, dropping a
/// degenerate segment left a gap, the two segments either side of it stopped
/// looking consecutive, and their shared vertex was reported as a self-tangency —
/// so a line with a duplicated vertex was wrongly called non-simple.
fn segments_of(mls: &MultiLineString<f64>) -> (Vec<Seg>, Vec<usize>) {
    let mut out = Vec::new();
    let mut last_index = Vec::with_capacity(mls.0.len());

    for (component, ls) in mls.0.iter().enumerate() {
        let mut index = 0usize;
        for line in ls.lines() {
            if line.start != line.end {
                out.push(Seg {
                    line,
                    component,
                    index,
                });
                index += 1;
            }
        }
        last_index.push(index.saturating_sub(1));
    }
    (out, last_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{MultiPoint, MultiPolygon, Point, Polygon};

    fn ls(coords: Vec<(f64, f64)>) -> Geometry<f64> {
        Geometry::LineString(LineString::from(coords))
    }

    #[test]
    fn a_plain_open_line_is_simple() {
        assert!(is_simple(&ls(vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)])));
    }

    /// The headline fix: a figure-eight is not simple. The old implementation
    /// returned `true` here.
    #[test]
    fn a_self_crossing_line_is_not_simple() {
        assert!(!is_simple(&ls(vec![
            (0.0, 0.0),
            (2.0, 2.0),
            (2.0, 0.0),
            (0.0, 2.0)
        ])));
    }

    #[test]
    fn a_closed_ring_is_simple_despite_its_coincident_endpoints() {
        assert!(is_simple(&ls(vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (0.0, 0.0)
        ])));
    }

    /// A loop that returns to an *interior* vertex is self-tangent, not simple —
    /// the closed-ring exemption must not be over-general.
    #[test]
    fn a_loop_touching_an_earlier_vertex_is_not_simple() {
        assert!(!is_simple(&ls(vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 1.0),
            (1.0, 2.0),
            (1.0, 0.0)
        ])));
    }

    /// A spike doubles back along itself: a collinear overlap between
    /// consecutive segments, which the "consecutive segments may touch" rule must
    /// not excuse.
    #[test]
    fn a_spike_that_doubles_back_is_not_simple() {
        assert!(!is_simple(&ls(vec![(0.0, 0.0), (2.0, 0.0), (1.0, 0.0)])));
    }

    #[test]
    fn a_repeated_vertex_is_tolerated_rather_than_treated_as_an_anomaly() {
        assert!(is_simple(&ls(vec![
            (0.0, 0.0),
            (1.0, 1.0),
            (1.0, 1.0),
            (2.0, 2.0)
        ])));
    }

    #[test]
    fn multilinestring_components_may_meet_at_their_ends_but_not_cross() {
        let end_to_end = Geometry::MultiLineString(MultiLineString(vec![
            LineString::from(vec![(0.0, 0.0), (1.0, 0.0)]),
            LineString::from(vec![(1.0, 0.0), (2.0, 0.0)]),
        ]));
        assert!(is_simple(&end_to_end));

        let crossing = Geometry::MultiLineString(MultiLineString(vec![
            LineString::from(vec![(0.0, 0.0), (2.0, 0.0)]),
            LineString::from(vec![(1.0, -1.0), (1.0, 1.0)]),
        ]));
        assert!(!is_simple(&crossing));

        // Touching the middle of another component is a tangency, not an
        // end-to-end join.
        let tangent = Geometry::MultiLineString(MultiLineString(vec![
            LineString::from(vec![(0.0, 0.0), (2.0, 0.0)]),
            LineString::from(vec![(1.0, 0.0), (1.0, 1.0)]),
        ]));
        assert!(!is_simple(&tangent));
    }

    #[test]
    fn multipoint_simplicity_is_about_duplicates() {
        assert!(is_simple(&Geometry::MultiPoint(MultiPoint(vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 1.0)
        ]))));
        assert!(!is_simple(&Geometry::MultiPoint(MultiPoint(vec![
            Point::new(0.0, 0.0),
            Point::new(0.0, 0.0)
        ]))));
        assert!(is_simple(&Geometry::Point(Point::new(0.0, 0.0))));
    }

    #[test]
    fn a_bowtie_polygon_is_reported_as_not_simple() {
        let bowtie = Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (2.0, 2.0),
                (2.0, 0.0),
                (0.0, 2.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(!is_simple(&Geometry::Polygon(bowtie.clone())));
        assert!(!is_simple(&Geometry::MultiPolygon(MultiPolygon(vec![
            bowtie
        ]))));

        let clean = Polygon::new(
            LineString::from(vec![
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
                (0.0, 0.0),
            ]),
            vec![],
        );
        assert!(is_simple(&Geometry::Polygon(clean)));
    }

    #[test]
    fn a_collection_is_simple_only_if_every_member_is() {
        let good = Geometry::GeometryCollection(
            vec![
                Geometry::Point(Point::new(0.0, 0.0)),
                ls(vec![(0.0, 0.0), (1.0, 1.0)]),
            ]
            .into(),
        );
        assert!(is_simple(&good));

        let bad = Geometry::GeometryCollection(
            vec![ls(vec![(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)])].into(),
        );
        assert!(!is_simple(&bad));
    }

    #[test]
    fn degenerate_input_is_simple_rather_than_an_error() {
        assert!(is_simple(&Geometry::GeometryCollection(Default::default())));
        assert!(is_simple(&ls(vec![])));
        assert!(is_simple(&ls(vec![(0.0, 0.0)])));
    }
}
