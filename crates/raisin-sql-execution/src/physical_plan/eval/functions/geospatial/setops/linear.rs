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

//! Set operations between 1-dimensional geometries.
//!
//! `geo` has no trait for this: `BooleanOps` covers `Polygon`/`MultiPolygon`
//! only, and `BooleanOps::clip` handles line-versus-*polygon* but not
//! line-versus-line. So `ST_INTERSECTION(line, line)` and
//! `ST_DIFFERENCE(line, line)` are built here on `geo::line_intersection`, which
//! gives exactly the two cases that matter: a `Collinear` overlap (a shared
//! sub-segment, 1-D) and a `SinglePoint` crossing (0-D).
//!
//! Everything is done by parameterising along the subject segment, so the result
//! keeps the subject's own vertices and direction rather than being rebuilt from
//! floating-point intersections.

use geo::line_intersection::{line_intersection, LineIntersection};
use geo::{Coord, Line, LineString, MultiLineString, MultiPoint, Point};

/// Where two linear geometries meet: the shared sub-segments and the isolated
/// crossing points.
///
/// A crossing point that already lies on a shared sub-segment is omitted, so the
/// two components of the result never overlap — an intersection result must not
/// report the same location at two dimensions.
pub(super) fn intersection(
    a: &MultiLineString<f64>,
    b: &MultiLineString<f64>,
) -> (MultiLineString<f64>, MultiPoint<f64>) {
    let subject: Vec<Line<f64>> = segments(a);
    let clip: Vec<Line<f64>> = segments(b);

    let mut shared: Vec<Line<f64>> = Vec::new();
    let mut crossings: Vec<Coord<f64>> = Vec::new();

    for s in &subject {
        for c in &clip {
            match line_intersection(*s, *c) {
                Some(LineIntersection::Collinear { intersection }) => shared.push(intersection),
                Some(LineIntersection::SinglePoint { intersection, .. }) => {
                    crossings.push(intersection)
                }
                None => {}
            }
        }
    }

    crossings.retain(|p| !shared.iter().any(|line| on_segment(*line, *p)));
    dedup_coords(&mut crossings);

    (
        MultiLineString(stitch(shared)),
        MultiPoint(crossings.into_iter().map(Point::from).collect()),
    )
}

/// `a` with every part that `b` also covers removed.
///
/// Only `Collinear` overlaps remove length: a transversal crossing is a
/// 0-dimensional intersection and takes nothing away from a 1-dimensional
/// geometry, which is why a difference can leave the subject untouched even
/// though the two geometries do intersect.
pub(super) fn difference(
    a: &MultiLineString<f64>,
    b: &MultiLineString<f64>,
) -> MultiLineString<f64> {
    let clip: Vec<Line<f64>> = segments(b);
    let mut survivors: Vec<Line<f64>> = Vec::new();

    for line in a {
        for s in line.lines() {
            survivors.extend(subtract(s, &clip));
        }
    }
    MultiLineString(stitch(survivors))
}

/// Remove from `s` every interval that a clip segment covers collinearly.
fn subtract(s: Line<f64>, clip: &[Line<f64>]) -> Vec<Line<f64>> {
    let mut covered: Vec<(f64, f64)> = Vec::new();
    for c in clip {
        if let Some(LineIntersection::Collinear { intersection }) = line_intersection(s, *c) {
            let (t0, t1) = (param(s, intersection.start), param(s, intersection.end));
            covered.push((t0.min(t1), t0.max(t1)));
        }
    }
    if covered.is_empty() {
        return vec![s];
    }

    covered.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(covered.len());
    for (lo, hi) in covered {
        match merged.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }

    // Emit the complement of the merged cover, dropping zero-length remnants.
    let mut out = Vec::new();
    let mut cursor = 0.0f64;
    for (lo, hi) in merged {
        if lo - cursor > T_EPSILON {
            out.push(Line::new(at(s, cursor), at(s, lo)));
        }
        cursor = cursor.max(hi);
    }
    if 1.0 - cursor > T_EPSILON {
        out.push(Line::new(at(s, cursor), s.end));
    }
    out
}

/// Below this fraction of a segment, a remnant is float noise rather than
/// geometry. Relative to the segment, so it is scale-independent.
const T_EPSILON: f64 = 1e-12;

/// Position of `p` along `s` as a fraction in `[0, 1]`.
///
/// Projects onto the longer axis, which avoids dividing by a near-zero delta on
/// an axis-aligned segment.
fn param(s: Line<f64>, p: Coord<f64>) -> f64 {
    let (dx, dy) = (s.end.x - s.start.x, s.end.y - s.start.y);
    if dx.abs() >= dy.abs() {
        if dx == 0.0 {
            0.0
        } else {
            ((p.x - s.start.x) / dx).clamp(0.0, 1.0)
        }
    } else {
        ((p.y - s.start.y) / dy).clamp(0.0, 1.0)
    }
}

fn at(s: Line<f64>, t: f64) -> Coord<f64> {
    Coord {
        x: s.start.x + (s.end.x - s.start.x) * t,
        y: s.start.y + (s.end.y - s.start.y) * t,
    }
}

fn on_segment(s: Line<f64>, p: Coord<f64>) -> bool {
    let t = param(s, p);
    let q = at(s, t);
    (q.x - p.x).abs() < 1e-12 && (q.y - p.y).abs() < 1e-12
}

fn segments(m: &MultiLineString<f64>) -> Vec<Line<f64>> {
    m.iter().flat_map(|ls| ls.lines()).collect()
}

/// Join segments that share an endpoint back into `LineString`s.
///
/// Without this, the difference of two long lines would return one `LineString`
/// per surviving segment, which is technically the same geometry but a needlessly
/// fragmented spelling of it.
fn stitch(mut segments: Vec<Line<f64>>) -> Vec<LineString<f64>> {
    segments.retain(|s| s.start != s.end);
    let mut out: Vec<LineString<f64>> = Vec::new();

    for s in segments {
        match out.last_mut() {
            Some(open) if open.0.last() == Some(&s.start) => open.0.push(s.end),
            _ => out.push(LineString::from(vec![s.start, s.end])),
        }
    }
    out
}

fn dedup_coords(coords: &mut Vec<Coord<f64>>) {
    let mut seen: Vec<(u64, u64)> = Vec::with_capacity(coords.len());
    coords.retain(|c| {
        let key = (c.x.to_bits(), c.y.to_bits());
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
    use geo::Length;

    fn ls(coords: Vec<(f64, f64)>) -> MultiLineString<f64> {
        MultiLineString(vec![LineString::from(coords)])
    }

    #[test]
    fn a_transversal_crossing_is_a_point_not_a_line() {
        let a = ls(vec![(0.0, 0.0), (2.0, 0.0)]);
        let b = ls(vec![(1.0, -1.0), (1.0, 1.0)]);
        let (lines, points) = intersection(&a, &b);
        assert!(lines.0.is_empty(), "a crossing shares no length");
        assert_eq!(points.0.len(), 1);
        assert_eq!(points.0[0], Point::new(1.0, 0.0));
    }

    #[test]
    fn an_overlap_is_a_line_and_the_endpoints_are_not_also_reported_as_points() {
        let a = ls(vec![(0.0, 0.0), (4.0, 0.0)]);
        let b = ls(vec![(1.0, 0.0), (3.0, 0.0)]);
        let (lines, points) = intersection(&a, &b);
        assert_eq!(lines.0.len(), 1);
        assert!((geo::Euclidean.length(&lines) - 2.0).abs() < 1e-12);
        assert!(
            points.0.is_empty(),
            "the shared sub-segment already covers its own endpoints"
        );
    }

    /// The defining property of a 1-D difference: a crossing removes nothing.
    #[test]
    fn a_crossing_removes_no_length() {
        let a = ls(vec![(0.0, 0.0), (2.0, 0.0)]);
        let b = ls(vec![(1.0, -1.0), (1.0, 1.0)]);
        let d = difference(&a, &b);
        assert!((geo::Euclidean.length(&d) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn an_interior_overlap_splits_the_subject_in_two() {
        let a = ls(vec![(0.0, 0.0), (4.0, 0.0)]);
        let b = ls(vec![(1.0, 0.0), (3.0, 0.0)]);
        let d = difference(&a, &b);
        assert_eq!(d.0.len(), 2, "0..1 and 3..4");
        assert!((geo::Euclidean.length(&d) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_full_overlap_removes_everything() {
        let a = ls(vec![(0.0, 0.0), (4.0, 0.0)]);
        assert!(difference(&a, &a).0.is_empty());
    }

    /// Overlapping cutters must merge, or the complement double-counts the gap.
    #[test]
    fn adjacent_cutters_merge_into_one_covered_interval() {
        let a = ls(vec![(0.0, 0.0), (10.0, 0.0)]);
        let cutters = MultiLineString(vec![
            LineString::from(vec![(1.0, 0.0), (5.0, 0.0)]),
            LineString::from(vec![(4.0, 0.0), (8.0, 0.0)]),
        ]);
        let d = difference(&a, &cutters);
        assert_eq!(d.0.len(), 2, "0..1 and 8..10");
        assert!((geo::Euclidean.length(&d) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn surviving_segments_are_stitched_back_into_one_linestring() {
        // Nothing is removed, so the four segments must come back as one line.
        let a = ls(vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (3.0, 0.0),
            (4.0, 0.0),
        ]);
        let d = difference(&a, &ls(vec![(0.0, 5.0), (1.0, 5.0)]));
        assert_eq!(d.0.len(), 1, "expected one stitched line, got {:?}", d.0);
        assert_eq!(d.0[0].0.len(), 5);
    }

    #[test]
    fn a_vertical_segment_parameterises_on_the_y_axis() {
        // dx == 0: projecting on x would divide by zero.
        let a = ls(vec![(0.0, 0.0), (0.0, 4.0)]);
        let b = ls(vec![(0.0, 1.0), (0.0, 3.0)]);
        let d = difference(&a, &b);
        assert_eq!(d.0.len(), 2);
        assert!((geo::Euclidean.length(&d) - 2.0).abs() < 1e-9);
    }
}
