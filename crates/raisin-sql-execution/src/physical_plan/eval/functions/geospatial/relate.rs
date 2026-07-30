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

//! The one code path behind all ten topological predicates.
//!
//! # Why one path
//!
//! Every topological predicate used to hand-roll a `match` arm per geometry-type
//! *pair*, over converters that existed only for `Point`, `LineString` and
//! `Polygon`. Three consequences followed mechanically, and all three were real
//! bugs:
//!
//! * each predicate supported a *different*, incomplete set of type pairs —
//!   `ST_DISJOINT` handled nine, `ST_CONTAINS` two, so `ST_DISJOINT` was not the
//!   complement of `ST_INTERSECTS`;
//! * no `Multi*` or `GeometryCollection` input worked anywhere, because no
//!   converter for them existed;
//! * the arms that could not be written by hand became `false` — `ST_CROSSES`
//!   and `ST_TOUCHES` hardcoded `false` for any `Point` argument, `ST_OVERLAPS`
//!   had a silent catch-all — which is a *wrong answer*, not a missing feature.
//!
//! `geo`'s DE-9IM [`Relate`] is implemented for `Geometry<f64>` itself, so a
//! single conversion plus one `relate` call yields an [`IntersectionMatrix`] that
//! is correct for **every** type pair, including nested `GeometryCollection`s.
//! Each predicate is then one method on that matrix. There is nowhere left for a
//! per-pair gap to hide, and the predicates are mutually consistent by
//! construction: `is_intersects` is *defined* as `!is_disjoint` inside `geo`.
//!
//! # Semantics
//!
//! DE-9IM is **planar** in the geometry's own coordinate space. On EPSG:4326 that
//! means edges are straight lines in lon/lat degrees, not great circles — the
//! same model PostGIS's `geometry` type uses, with the same documented weakness
//! near the poles and across the antimeridian. Measurements (`ST_DISTANCE`,
//! `ST_AREA`, …) are geodesic; predicates are not. See the CRS documentation.
//!
//! Altitude is ignored, exactly as PostGIS's 2-D predicates ignore it. The
//! Z-aware functions live in `z_support` and read Z off the JSON.
//!
//! # Empty geometries
//!
//! Handled by `geo`, not by us, and it gets them right:
//! `GeometryGraph::add_geometry` returns early for an empty component, so the
//! matrix stays `empty_disjoint`. That yields `ST_DISJOINT = true`,
//! `ST_INTERSECTS = false`, every other predicate `false`, and — via an explicit
//! special case in `is_equal_topo` — `ST_EQUALS(empty, empty) = true`. Those are
//! the JTS/PostGIS answers.

use std::borrow::Cow;

use geo::relate::IntersectionMatrix;
use geo::Relate;
use raisin_error::Error;
use raisin_geometry::Geom;
use raisin_sql::analyzer::{Literal, TypedExpr};
use serde_json::Value;

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::executor::Row;

/// Extract the GeoJSON value from an evaluated argument.
///
/// `Ok(None)` means SQL `NULL`. `TEXT` is accepted and parsed so that a string
/// literal or a `ST_ASGEOJSON` result composes; anything else is a type error
/// naming the function, because a predicate that quietly returned `false` for a
/// mistyped argument is exactly the class of defect this module removes.
fn operand<'a>(fn_name: &str, lit: &'a Literal) -> Result<Option<Cow<'a, Value>>, Error> {
    match lit {
        Literal::Null => Ok(None),
        Literal::Geometry(v) | Literal::JsonB(v) => Ok(Some(Cow::Borrowed(v))),
        Literal::Text(s) => {
            let v: Value = serde_json::from_str(s).map_err(|e| {
                Error::Validation(format!(
                    "{fn_name}: argument is TEXT but is not parseable GeoJSON: {e}"
                ))
            })?;
            Ok(Some(Cow::Owned(v)))
        }
        other => Err(Error::Validation(format!(
            "{fn_name} requires GEOMETRY arguments, got {:?}",
            other.data_type()
        ))),
    }
}

/// The geometry a NULL-evaluating operand reaches through a nested property path.
///
/// `Ok(None)` when the operand is not NULL (the ordinary literal is used
/// instead), or when the path matches nothing.
fn nested_operand(
    fn_name: &str,
    literal: &Literal,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<serde_json::Value>, Error> {
    if !matches!(literal, Literal::Null) {
        return Ok(None);
    }
    let mut matched = super::property_path::resolve_from_row(&args[index], row);
    match matched.len() {
        0 => Ok(None),
        1 => Ok(Some(matched.remove(0).geometry)),
        n => Err(Error::Validation(format!(
            "{fn_name}: argument {} names {n} geometries via a wildcard property path; \
             only ST_DWITHIN and ST_DISTANCE define an answer over several geometries \
             (any-within and minimum-distance respectively). Name one concrete path, \
             e.g. properties->>'stops.0.geo'.",
            index + 1
        ))),
    }
}

/// Resolve two arguments to `geo` geometries in a common CRS.
///
/// `Ok(None)` is the SQL `NULL` short circuit. Both arguments are evaluated
/// before either is type-checked, so `f(<not a geometry>, NULL)` propagates
/// `NULL` rather than erroring — matching the behaviour these functions had
/// before, and matching SQL's general treatment of `NULL`.
///
/// SRID handling follows [`raisin_geometry::resolve_pair_srid`]: an unlabelled
/// operand adopts the other's SRID (which is what keeps every existing 4326
/// query working unchanged), and two *different* explicit SRIDs are a hard error
/// telling the user to wrap one side in `ST_TRANSFORM`. An implicit transform
/// would hide a data-modelling mistake and, worse, would make the query's
/// success depend on which Cargo features the server was built with.
pub(super) fn resolve_pair(
    fn_name: &str,
    args: &[TypedExpr],
    row: &Row,
) -> Result<Option<(Geom, Geom)>, Error> {
    if args.len() != 2 {
        return Err(Error::Validation(format!(
            "{fn_name} requires exactly 2 arguments"
        )));
    }

    let a_lit = eval_expr(&args[0], row)?;
    let b_lit = eval_expr(&args[1], row)?;

    // A NULL operand may be a NESTED property path the ordinary JSON lookup
    // cannot reach (`properties->>'venue.geo'`). Resolving it here rather than
    // short-circuiting is what makes the DE-9IM predicates work on nested
    // geometry at all — without it they answer NULL, which reads as "no match"
    // and is the silent-empty failure this subsystem exists to avoid. It is also
    // where a WILDCARD path is rejected: these predicates have no defined answer
    // over a set of geometries.
    let a_nested = nested_operand(fn_name, &a_lit, args, 0, row)?;
    let b_nested = nested_operand(fn_name, &b_lit, args, 1, row)?;

    // The NULL short circuit, restored AFTER nested resolution and BEFORE any
    // type check — so `f(<not a geometry>, NULL)` still propagates NULL rather
    // than erroring, exactly as it did before nested paths existed.
    let a_null = matches!(a_lit, Literal::Null) && a_nested.is_none();
    let b_null = matches!(b_lit, Literal::Null) && b_nested.is_none();
    if a_null || b_null {
        return Ok(None);
    }

    let a_val = match a_nested {
        Some(v) => Cow::Owned(v),
        None => match operand(fn_name, &a_lit)? {
            Some(v) => v,
            None => return Ok(None),
        },
    };
    let b_val = match b_nested {
        Some(v) => Cow::Owned(v),
        None => match operand(fn_name, &b_lit)? {
            Some(v) => v,
            None => return Ok(None),
        },
    };

    // Rejects two different explicit SRIDs before any coordinate is trusted.
    raisin_geometry::resolve_pair_srid(fn_name, &a_val, &b_val, None)?;

    let a = raisin_geometry::to_geo(&a_val, None)?;
    let b = raisin_geometry::to_geo(&b_val, None)?;
    Ok(Some((a, b)))
}

/// The DE-9IM intersection matrix of two resolved geometries.
///
/// Every coordinate is finite here — [`raisin_geometry::to_geo`] rejects
/// non-finite ordinates at the boundary — which is the precondition `geo`'s
/// `Relate` documents ("must not be called on geometries containing `NaN`").
#[inline]
pub(super) fn matrix(a: &Geom, b: &Geom) -> IntersectionMatrix {
    a.geometry.relate(&b.geometry)
}

/// Evaluate one topological predicate: resolve, relate, read the matrix.
///
/// The `test` argument is an [`IntersectionMatrix`] method, so a predicate
/// implementation is a single call and cannot drift from the DE-9IM definition.
#[inline]
pub(super) fn predicate(
    fn_name: &str,
    args: &[TypedExpr],
    row: &Row,
    test: fn(&IntersectionMatrix) -> bool,
) -> Result<Literal, Error> {
    match resolve_pair(fn_name, args, row)? {
        None => Ok(Literal::Null),
        Some((a, b)) => Ok(Literal::Boolean(test(&matrix(&a, &b)))),
    }
}

/// Validate a DE-9IM pattern up front, over all nine positions.
///
/// `geo`'s [`IntersectionMatrix::matches`] parses lazily and returns `Ok(false)`
/// as soon as one position fails, so it never looks at the characters after it:
/// `matches("TTTTTTTTX")` against a matrix that fails at position 1 answers
/// `false` rather than reporting the invalid `X`. A typo late in a pattern would
/// therefore read as a legitimate negative. It also measures length in *bytes*,
/// so a nine-character pattern containing a multi-byte character is misreported.
/// Both are checked here instead, before delegating.
pub(super) fn validate_de9im_pattern(fn_name: &str, spec: &str) -> Result<(), Error> {
    let mut count = 0usize;
    for c in spec.chars() {
        count += 1;
        if !matches!(c, '*' | 't' | 'T' | 'f' | 'F' | '0' | '1' | '2') {
            return Err(Error::Validation(format!(
                "{fn_name}: invalid DE-9IM pattern {spec:?}: character {c:?} is not one of \
                 * (any), T (non-empty), F (empty), 0, 1, 2"
            )));
        }
    }
    if count != 9 {
        return Err(Error::Validation(format!(
            "{fn_name}: invalid DE-9IM pattern {spec:?}: expected exactly 9 characters, got {count}"
        )));
    }
    Ok(())
}

/// Render a matrix as its nine-character DE-9IM string, row-major in
/// interior/boundary/exterior order.
///
/// `geo` only exposes this shape through `Debug` (which wraps it in
/// `IntersectionMatrix(...)`), so it is rebuilt here from the public
/// [`IntersectionMatrix::get`] accessor.
pub(super) fn matrix_to_de9im(m: &IntersectionMatrix) -> String {
    use geo::coordinate_position::CoordPos;
    use geo::dimensions::Dimensions;

    const POSITIONS: [CoordPos; 3] = [CoordPos::Inside, CoordPos::OnBoundary, CoordPos::Outside];
    let mut out = String::with_capacity(9);
    for a in POSITIONS {
        for b in POSITIONS {
            out.push(match m.get(a, b) {
                Dimensions::Empty => 'F',
                Dimensions::ZeroDimensional => '0',
                Dimensions::OneDimensional => '1',
                Dimensions::TwoDimensional => '2',
            });
        }
    }
    out
}
