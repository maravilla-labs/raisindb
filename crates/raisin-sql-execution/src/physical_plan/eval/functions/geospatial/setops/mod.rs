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

//! `ST_UNION` / `ST_INTERSECTION` / `ST_DIFFERENCE` / `ST_SYMDIFFERENCE` over
//! **every** pair of geometry types.
//!
//! # Why this is not one call to `BooleanOps`
//!
//! `geo::BooleanOps` is implemented for `Polygon` and `MultiPolygon` only. The
//! previous implementation therefore accepted Polygon+Polygon (and Point+Point for
//! union) and returned "not supported" for everything else — including the case
//! the brief calls out, `ST_AREA(ST_UNION(a, b))` failing whenever the union
//! yielded a MultiPolygon.
//!
//! The fix is to operate per dimension and recombine ([`parts`]):
//!
//! | dimension | union / intersection / difference |
//! |---|---|
//! | 2-D vs 2-D | `BooleanOps` |
//! | 1-D vs 2-D | `BooleanOps::clip`, which polygon-clips lines |
//! | 1-D vs 1-D | [`linear`], built on `geo::line_intersection` |
//! | 0-D vs anything | DE-9IM coverage tests via `Relate` |
//!
//! # The overlay rule, stated once
//!
//! A result never reports the same location at two dimensions. A line inside a
//! resulting polygon is absorbed by it; a point on a resulting line is absorbed by
//! that line. This is what OGC calls a *simple* result and it is why
//! `ST_UNION(polygon, a_line_through_it)` is just the polygon.

mod linear;
mod parts;

use geo::{BooleanOps, Geometry, HasDimensions, MultiLineString, MultiPoint, MultiPolygon, Relate};
use raisin_error::Error;
use raisin_geometry::Geom;

use parts::{decompose, recompose, Parts};

/// The four OGC overlay operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SetOp {
    Union,
    Intersection,
    Difference,
    SymDifference,
}

impl SetOp {
    pub(super) fn name(self) -> &'static str {
        match self {
            SetOp::Union => "ST_UNION",
            SetOp::Intersection => "ST_INTERSECTION",
            SetOp::Difference => "ST_DIFFERENCE",
            SetOp::SymDifference => "ST_SYMDIFFERENCE",
        }
    }
}

/// Apply a set operation, in the operands' shared coordinate space.
///
/// Both operands must already be in the same CRS; the result carries it. Planar,
/// like PostGIS's `geometry` type — an overlay in lon/lat degrees uses straight
/// edges in lon/lat space, not great circles.
pub(super) fn apply(op: SetOp, a: &Geom, b: &Geom) -> Result<Geom, Error> {
    raisin_geometry::require_same_srid(op.name(), a, b)?;

    let (pa, pb) = (decompose(&a.geometry), decompose(&b.geometry));

    let result = match op {
        SetOp::Union => union(&pa, &pb),
        SetOp::Intersection => intersection(&pa, &pb),
        SetOp::Difference => difference(&pa, &pb),
        SetOp::SymDifference => {
            let mut left = difference(&pa, &pb);
            let right = difference(&pb, &pa);
            left.polygons = left.polygons.union(&right.polygons);
            left.lines.0.extend(right.lines.0);
            left.points.0.extend(right.points.0);
            left
        }
    };

    Ok(a.map_geometry(absorb(result)))
}

/// The n-way union of several geometries, used by aggregate contexts.
///
/// `geo::unary_union` is the efficient path for the polygonal part; the other
/// dimensions accumulate.
pub(super) fn union_all<'a>(geoms: impl IntoIterator<Item = &'a Geom>) -> Option<Geom> {
    let mut iter = geoms.into_iter();
    let first = iter.next()?;
    let mut acc = first.clone();
    for next in iter {
        acc = apply(SetOp::Union, &acc, next).unwrap_or(acc);
    }
    Some(acc)
}

/// Union, dimension by dimension.
///
/// The 1-D and 0-D halves must SUBTRACT before they concatenate. Plain
/// concatenation is not a union: it double-counts whatever the two operands
/// share, so `ST_LENGTH(ST_UNION(a, b))` over two overlapping collinear lines
/// reported the sum of their lengths rather than the length of their union
/// (measured: two 2-degree segments overlapping by 1 degree gave 4 degrees
/// instead of 3), and `ST_UNION` of a point with itself yielded a two-member
/// MultiPoint. Both broke the identity
/// `area(union) = area(intersection) + area(symdifference)` in the non-areal
/// case, which is how this was found.
///
/// `geo`'s `BooleanOps` covers the 2-D half; the 1-D half reuses
/// [`linear::difference`], which already removes exactly the collinear overlaps
/// (a transversal crossing is 0-dimensional and correctly removes no length).
fn union(a: &Parts, b: &Parts) -> Parts {
    let polygons = a.polygons.union(&b.polygons);

    // Keep all of A's lines, plus the parts of B that A does not already cover.
    let mut lines = a.lines.clone();
    lines.0.extend(linear::difference(&b.lines, &a.lines).0);

    // Same rule at 0-D: a location present in both operands appears once.
    let mut points = a.points.clone();
    for p in &b.points.0 {
        if !points.0.iter().any(|q| q == p) {
            points.0.push(*p);
        }
    }

    Parts {
        polygons,
        lines,
        points,
    }
}

fn intersection(a: &Parts, b: &Parts) -> Parts {
    // 2-D x 2-D.
    let polygons = a.polygons.intersection(&b.polygons);

    // 1-D x 2-D in both directions, plus 1-D x 1-D.
    let mut lines = MultiLineString(Vec::new());
    lines.0.extend(clip_inside(&b.polygons, &a.lines).0);
    lines.0.extend(clip_inside(&a.polygons, &b.lines).0);
    let (shared, crossings) = linear::intersection(&a.lines, &b.lines);
    lines.0.extend(shared.0);

    // 0-D x anything, in both directions, plus the 1-D crossing points.
    let mut points = covered_by(&a.points, &b.geometry());
    points.0.extend(covered_by(&b.points, &a.geometry()).0);
    points.0.extend(crossings.0);

    Parts {
        polygons,
        lines,
        points,
    }
}

fn difference(a: &Parts, b: &Parts) -> Parts {
    let polygons = a.polygons.difference(&b.polygons);

    // A's lines survive where they are outside B's area and not collinear with
    // B's lines.
    let outside_area = clip_outside(&b.polygons, &a.lines);
    let lines = linear::difference(&outside_area, &b.lines);

    let points = not_covered_by(&a.points, &b.geometry());

    Parts {
        polygons,
        lines,
        points,
    }
}

/// Absorb lower-dimensional parts that a higher-dimensional part already covers.
fn absorb(mut parts: Parts) -> Geometry<f64> {
    if !parts.polygons.0.is_empty() {
        parts.lines = clip_outside(&parts.polygons, &parts.lines);
    }
    if !parts.polygons.0.is_empty() || !parts.lines.0.is_empty() {
        let cover = Parts {
            polygons: parts.polygons.clone(),
            lines: parts.lines.clone(),
            points: MultiPoint(Vec::new()),
        }
        .geometry();
        parts.points = not_covered_by(&parts.points, &cover);
    }
    recompose(parts)
}

/// The part of `lines` inside `polygons` (`invert = false` in `geo`'s `clip`).
fn clip_inside(polygons: &MultiPolygon<f64>, lines: &MultiLineString<f64>) -> MultiLineString<f64> {
    if polygons.0.is_empty() || lines.0.is_empty() {
        return MultiLineString(Vec::new());
    }
    polygons.clip(lines, false)
}

/// The part of `lines` outside `polygons` (`invert = true`).
fn clip_outside(
    polygons: &MultiPolygon<f64>,
    lines: &MultiLineString<f64>,
) -> MultiLineString<f64> {
    if lines.0.is_empty() {
        return MultiLineString(Vec::new());
    }
    if polygons.0.is_empty() {
        return lines.clone();
    }
    polygons.clip(lines, true)
}

fn covered_by(points: &MultiPoint<f64>, target: &Geometry<f64>) -> MultiPoint<f64> {
    filter_coverage(points, target, true)
}

fn not_covered_by(points: &MultiPoint<f64>, target: &Geometry<f64>) -> MultiPoint<f64> {
    filter_coverage(points, target, false)
}

/// Keep the points whose coverage by `target` matches `want`.
///
/// `Relate` is undefined on an empty geometry, so emptiness is short-circuited:
/// nothing is covered by nothing.
fn filter_coverage(
    points: &MultiPoint<f64>,
    target: &Geometry<f64>,
    want: bool,
) -> MultiPoint<f64> {
    if points.0.is_empty() {
        return MultiPoint(Vec::new());
    }
    if target.is_empty() {
        return if want {
            MultiPoint(Vec::new())
        } else {
            points.clone()
        };
    }
    MultiPoint(
        points
            .0
            .iter()
            .filter(|p| p.relate(target).is_coveredby() == want)
            .copied()
            .collect(),
    )
}

impl Parts {
    /// The parts as one `geo` geometry, for the `Relate` coverage tests.
    fn geometry(&self) -> Geometry<f64> {
        recompose(self.clone())
    }
}

#[cfg(test)]
mod tests;
