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

//! The single doorway between a SQL `Literal` and a `geo` geometry.
//!
//! Every ST_\* function goes through [`geom_arg`] on the way in and
//! [`geom_result`] on the way out. That is what makes "accepts every geometry
//! type" a property of one function rather than a claim repeated forty times: the
//! conversion is `raisin_geometry::to_geo`, which delegates to `geojson` 1.0's
//! complete `TryFrom` impls, so `Multi*` and `GeometryCollection` arrive here
//! already handled.
//!
//! # Why binary functions must not each parse their own operands
//!
//! An SRID has to be resolved across *both* operands before either becomes a
//! [`Geom`]: the two have to end up in ONE frame, and which frame that is depends
//! on both labels. Parsing them independently would resolve each to EPSG:4326 and
//! silently lose a projected label, so [`geom_pair`] is the only correct way to
//! read two geometry arguments. An operand with no label is WGS84 and is
//! reprojected into the other's frame — see [`geom_pair`] for why relabelling it
//! instead was a silent wrong answer.

use geo::{Geometry, MultiPolygon};
use raisin_error::Error;
use raisin_geometry::{from_geo, to_geo, Geom};
use raisin_sql::analyzer::{Literal, TypedExpr};
use serde_json::Value;

use super::z_support::geometry_arg;
use crate::physical_plan::executor::Row;

/// Evaluate one argument into a [`Geom`].
///
/// `Ok(None)` means the argument was SQL NULL; every ST_\* function propagates
/// that as NULL rather than erroring.
pub(super) fn geom_arg(
    fn_name: &str,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<Geom>, Error> {
    match geometry_arg(fn_name, args, index, row)? {
        None => Ok(None),
        Some(value) => Ok(Some(to_geo(&value, None)?)),
    }
}

/// Evaluate two geometry arguments, resolving their CRS jointly.
///
/// Both operands come out in ONE CRS — the labelled one when exactly one carries
/// a label, so the result of a set operation keeps the CRS it has always had.
/// `SridMismatch` is still raised when both carry *different* explicit labels:
/// deliberately an error, as in PostGIS, because an implicit transform would hide
/// a data-modelling mistake and make the query's success depend on which Cargo
/// features the server was built with.
///
/// # An unlabelled operand is WGS84, and is REPROJECTED, not relabelled
///
/// "Unlabelled means WGS84 by convention" holds everywhere else in RaisinDB —
/// the spatial index normalises on exactly that rule. This function used to
/// *relabel* the unlabelled operand instead: `ST_DWITHIN(<a column stored in
/// EPSG:3857>, ST_POINT(8.54, 47.37), 5000)` read `(8.54, 47.37)` as 3857
/// eastings, i.e. a point in the Atlantic, and answered `false`.
///
/// That made the row-level answer disagree with the index's, which normalises
/// both sides to WGS84 and answers `true`. The disagreement was invisible while
/// the index was usable and surfaced the moment a query fell back to a scan —
/// caught by a concurrent reader during an index rebuild, as one wrong answer in
/// 106. Reprojecting keeps the output CRS identical to before while making the
/// two paths agree.
pub(super) fn geom_pair(
    fn_name: &str,
    args: &[TypedExpr],
    row: &Row,
) -> Result<Option<(Geom, Geom)>, Error> {
    let (Some(a), Some(b)) = (
        geometry_arg(fn_name, args, 0, row)?,
        geometry_arg(fn_name, args, 1, row)?,
    ) else {
        return Ok(None);
    };
    geom_pair_values(fn_name, &a, &b).map(Some)
}

/// [`geom_pair`] over two already-evaluated GeoJSON values.
///
/// Exists so the one CRS rule is applied in one place: `compute_haversine_distance`
/// takes raw values (it composes with a vertical leg read off the JSON) and used to
/// carry its own copy of the resolution, which is how the two could drift.
pub(super) fn geom_pair_values(fn_name: &str, a: &Value, b: &Value) -> Result<(Geom, Geom), Error> {
    // Errors on two different explicit labels; otherwise the CRS both operands
    // must end up in.
    let crs = raisin_geometry::resolve_pair_srid(fn_name, a, b, None)?;
    let a_label = raisin_geometry::srid_member(a)?;
    let b_label = raisin_geometry::srid_member(b)?;
    Ok((into_crs(a, a_label, crs)?, into_crs(b, b_label, crs)?))
}

/// Parse one operand and put it in `target`.
///
/// `label` is the operand's OWN explicit SRID member. `None` means unlabelled,
/// which is WGS84 — so an unlabelled operand facing a projected partner is parsed
/// as WGS84 and then REPROJECTED, never reinterpreted.
fn into_crs(
    value: &Value,
    label: Option<raisin_geometry::Crs>,
    target: raisin_geometry::Crs,
) -> Result<Geom, Error> {
    // Labelled: it is already in its own frame, and `resolve_pair_srid` has
    // guaranteed that frame IS the target (it errors otherwise).
    if label.is_some() || target == raisin_geometry::Crs::WGS84 {
        return Ok(to_geo(value, Some(target.srid()))?);
    }
    let wgs84 = to_geo(value, None)?;
    Ok(raisin_geometry::transform(&wgs84, target)?)
}

/// Serialize a [`Geom`] back into a geometry literal.
pub(super) fn geom_result(g: &Geom) -> Result<Literal, Error> {
    Ok(Literal::Geometry(from_geo(g)?))
}

/// Serialize a bare `geo` geometry, inheriting the CRS and vertical extent of the
/// geometry it was derived from.
pub(super) fn derived_result(geometry: Geometry<f64>, from: &Geom) -> Result<Literal, Error> {
    geom_result(&from.map_geometry(geometry))
}

/// The canonical empty geometry as a literal.
///
/// Emptiness *propagates* through every function instead of erroring — undefined
/// emptiness was the largest single source of spurious "unsupported geometry
/// type" errors in the previous implementation.
pub(super) fn empty_result() -> Literal {
    Literal::Geometry(raisin_geometry::empty())
}

/// Reduce a `MultiPolygon` to the narrowest type that represents it.
///
/// `geo`'s `BooleanOps` and `Buffer` always return a `MultiPolygon`, even for a
/// single ring. PostGIS returns the minimal type, and so do we: a one-polygon
/// result is a `Polygon`, and a zero-polygon result is the canonical empty
/// geometry rather than an empty `MultiPolygon`.
pub(super) fn narrow_multipolygon(mp: MultiPolygon<f64>) -> Geometry<f64> {
    let mut polygons = mp.0;
    match polygons.len() {
        0 => Geometry::GeometryCollection(Default::default()),
        1 => Geometry::Polygon(polygons.remove(0)),
        _ => Geometry::MultiPolygon(MultiPolygon(polygons)),
    }
}

/// Evaluate one argument as a raw GeoJSON `Value`, without converting to `geo`.
///
/// For the few functions that are defined on the *representation* rather than on
/// the geometry — `ST_ASGEOJSON` and the Z accessors, which need the third
/// ordinate `geo` cannot hold.
pub(super) fn value_arg(
    fn_name: &str,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<Value>, Error> {
    geometry_arg(fn_name, args, index, row)
}
