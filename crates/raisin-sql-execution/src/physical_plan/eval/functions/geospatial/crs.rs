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

//! Argument plumbing and coordinate walking for the three CRS functions.
//!
//! # The three functions, and the distinction that must never blur
//!
//! | Function | Coordinates | `srid` label | Meaning |
//! |----------|-------------|--------------|---------|
//! | `ST_SRID(g)` | untouched | read | "what CRS is this in?" |
//! | `ST_SETSRID(g, s)` | **untouched** | **overwritten** | "the label was wrong" |
//! | `ST_TRANSFORM(g, s)` | **recomputed** | overwritten | "put this in another CRS" |
//!
//! `ST_SETSRID` is a *reinterpretation* and `ST_TRANSFORM` is a *movement*. Using
//! `ST_SETSRID` where `ST_TRANSFORM` was meant silently relabels coordinates that
//! were never converted, which is the single most common way a multi-CRS dataset
//! becomes quietly wrong — so both functions say so in their own doc comments as
//! well as here.
//!
//! # Why the reprojection walks JSON instead of going through `geo`
//!
//! `geo_types::Coord` is strictly 2-D, so `to_geo` -> `transform` -> `from_geo`
//! would drop every altitude. All the transforms RaisinDB can perform are purely
//! horizontal (a UTM or Mercator projection says nothing about height), so
//! rewriting `(x, y)` in place and carrying any third or fourth ordinate through
//! verbatim is both cheaper and strictly more faithful. It also preserves
//! `GeometryCollection` nesting without a round trip.
//!
//! # Shared, not local
//!
//! Everything about the *carrier* — reading the `srid` member, the
//! unlabelled-adopts rule, mismatch wording — lives in `raisin_geometry::srid` so
//! that the ~20 binary ST_\* functions owned by other areas phrase it identically.
//! This module holds only what is specific to evaluating the CRS functions.

use raisin_error::Error;
use raisin_geometry::Crs;
use serde_json::Value;

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// The seven RFC 7946 geometry types, which are exactly the inputs the CRS
/// functions rewrite in place.
const GEOMETRY_TYPES: [&str; 7] = [
    "Point",
    "MultiPoint",
    "LineString",
    "MultiLineString",
    "Polygon",
    "MultiPolygon",
    "GeometryCollection",
];

/// Evaluate an argument as an SRID.
///
/// Accepts an integer (`4326`) or a textual CRS (`'EPSG:4326'`, `'SRID=4326'`,
/// `'urn:ogc:def:crs:EPSG::4326'`). `Ok(None)` means SQL NULL, which the caller
/// propagates.
///
/// Foreign authorities are rejected rather than reinterpreted: `'ESRI:102100'`
/// denotes WebMercator in ESRI's registry but is **not** EPSG:102100, and quietly
/// treating it as one would be exactly the class of lie this work removes.
pub(super) fn srid_arg(
    fn_name: &str,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<Crs>, Error> {
    let literal = eval_expr(&args[index], row)?;
    let code: i64 = match literal {
        Literal::Null => return Ok(None),
        Literal::Int(i) => i as i64,
        Literal::BigInt(i) => i,
        // A whole-valued double is accepted because a bound parameter or an
        // arithmetic expression easily produces one; a fractional SRID is not.
        Literal::Double(d) if d.fract() == 0.0 && d.is_finite() => d as i64,
        Literal::Text(s) => {
            return Crs::parse(&s)
                .map(Some)
                .map_err(|e| Error::Validation(format!("{fn_name}: {e}")))
        }
        other => {
            return Err(Error::Validation(format!(
                "{fn_name}: SRID must be an integer or a CRS string such as \
                 'EPSG:4326', got {:?}",
                other.data_type()
            )))
        }
    };

    let srid = u32::try_from(code).ok().filter(|c| *c > 0).ok_or_else(|| {
        Error::Validation(format!(
            "{fn_name}: {code} is not a valid SRID; EPSG codes are positive integers"
        ))
    })?;
    // `Crs::from` collapses the deprecated WebMercator aliases (3785, 900913)
    // onto 3857 so that `ST_TRANSFORM(g, 900913)` is a no-op rather than an
    // "unsupported pair".
    Ok(Some(Crs::from(srid)))
}

/// The effective CRS of a geometry value.
///
/// # Why `schema_default` is always `None` here
///
/// Expression evaluation sees a bare `serde_json::Value` in a [`Row`]; there is no
/// NodeType or workspace handle at this depth, so a schema-declared default SRID
/// cannot be consulted. That default is applied where it *is* available — on the
/// write path and in the spatial index — so an unlabelled stored geometry in a
/// workspace whose schema declares EPSG:2056 is indexed as 2056, while
/// `ST_SRID(properties->>'loc')` on that same value reports 4326. Closing that gap
/// needs schema context threaded into `Row`; it is tracked as a follow-up rather
/// than papered over with a guess.
pub(super) fn effective_srid(fn_name: &str, value: &Value) -> Result<Crs, Error> {
    raisin_geometry::srid_of(value, None).map_err(|e| Error::Validation(format!("{fn_name}: {e}")))
}

/// Validate that `value` is a GeoJSON geometry and return its type name.
///
/// `Feature` and `FeatureCollection` are reported as such so the caller can route
/// them through the `geo` path; anything else is an error naming what was found.
pub(super) fn geometry_type_of<'a>(fn_name: &str, value: &'a Value) -> Result<&'a str, Error> {
    let type_name = value.get("type").and_then(Value::as_str).ok_or_else(|| {
        Error::Validation(format!("{fn_name}: value has no GeoJSON 'type' member"))
    })?;

    if GEOMETRY_TYPES.contains(&type_name)
        || type_name == "Feature"
        || type_name == "FeatureCollection"
    {
        Ok(type_name)
    } else {
        Err(Error::Validation(format!(
            "{fn_name}: '{type_name}' is not a GeoJSON geometry type"
        )))
    }
}

/// Reproject every position of a GeoJSON geometry from `from` to `to`.
///
/// All-or-nothing: one coordinate outside the target CRS's domain fails the whole
/// geometry rather than emitting a half-projected ring, which would be a valid
/// looking geometry describing nowhere.
///
/// Third and fourth ordinates ride through untouched — every transform available
/// here is horizontal, so altitude is unaffected by definition.
pub(super) fn reproject_geometry(
    fn_name: &str,
    value: &Value,
    from: Crs,
    to: Crs,
) -> Result<Value, Error> {
    // A Feature carries properties we have no business rewriting, and `geojson`'s
    // own conversion already defines Feature -> geometry, so route those through
    // `raisin_geometry`. Altitude is lost on that path (geo is 2-D); a 3-D Feature
    // is a rare enough input to be worth the simpler code, and it is documented.
    if matches!(
        geometry_type_of(fn_name, value)?,
        "Feature" | "FeatureCollection"
    ) {
        let geom = raisin_geometry::to_geo(value, None)
            .map_err(|e| Error::Validation(format!("{fn_name}: {e}")))?;
        let moved = raisin_geometry::transform(&geom, to)
            .map_err(|e| Error::Validation(format!("{fn_name}: {e}")))?;
        return raisin_geometry::from_geo(&moved)
            .map_err(|e| Error::Validation(format!("{fn_name}: {e}")));
    }

    let moved = reproject_coordinates(fn_name, value, from, to)?;

    // Stamp the new label, at the TOP LEVEL ONLY. `srid` is read from the outermost
    // object (see `raisin_geometry::srid_member`), so labelling the members of a
    // GeometryCollection would add noise that nothing reads. `with_srid` removes
    // the member for WGS84, keeping 4326 output strictly RFC-7946-conformant — the
    // RFC mandates 4326 and forbids declaring another CRS, so our `srid` member is
    // a documented extension used only when it must be.
    Ok(raisin_geometry::with_srid(moved, to))
}

/// Rewrite every position of a geometry, leaving the `srid` member alone.
fn reproject_coordinates(fn_name: &str, value: &Value, from: Crs, to: Crs) -> Result<Value, Error> {
    let type_name = geometry_type_of(fn_name, value)?;
    let mut out = value.clone();

    if type_name == "GeometryCollection" {
        let members = out
            .get("geometries")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::Validation(format!("{fn_name}: GeometryCollection has no 'geometries'"))
            })?
            .clone();
        let moved = members
            .iter()
            .map(|member| reproject_coordinates(fn_name, member, from, to))
            .collect::<Result<Vec<_>, Error>>()?;
        out["geometries"] = Value::Array(moved);
    } else {
        let coords = out
            .get("coordinates")
            .ok_or_else(|| {
                Error::Validation(format!("{fn_name}: {type_name} has no 'coordinates'"))
            })?
            .clone();
        out["coordinates"] = map_positions(fn_name, &coords, from, to)?;
    }

    Ok(out)
}

/// Recurse through the arbitrarily nested `coordinates` of any geometry type.
///
/// A position is recognised structurally — an array whose first element is a
/// number — so one walker serves Point, LineString, Polygon and every `Multi*`
/// without a per-type arm, which is precisely the kind of duplication that left
/// `Multi*` unsupported across the ST_\* family before `raisin-geometry` existed.
fn map_positions(fn_name: &str, node: &Value, from: Crs, to: Crs) -> Result<Value, Error> {
    let items = node.as_array().ok_or_else(|| {
        Error::Validation(format!(
            "{fn_name}: expected a coordinate array, got {node}"
        ))
    })?;

    match items.first() {
        // A position. Everything from index 2 on (altitude, and an M ordinate if
        // some producer emitted one) is carried through verbatim.
        Some(first) if first.is_number() => {
            if items.len() < 2 {
                return Err(Error::Validation(format!(
                    "{fn_name}: position {node} has fewer than two ordinates"
                )));
            }
            let x = ordinate(fn_name, &items[0])?;
            let y = ordinate(fn_name, &items[1])?;
            let (nx, ny) = raisin_proj::transform_coord(from, to, x, y)
                .map_err(|e| Error::Validation(format!("{fn_name}: {e}")))?;

            let mut position = Vec::with_capacity(items.len());
            position.push(number(fn_name, nx)?);
            position.push(number(fn_name, ny)?);
            position.extend(items[2..].iter().cloned());
            Ok(Value::Array(position))
        }
        // An empty ring/geometry, or a deeper level of nesting. Both recurse
        // correctly: mapping an empty slice yields an empty array.
        _ => items
            .iter()
            .map(|child| map_positions(fn_name, child, from, to))
            .collect::<Result<Vec<_>, Error>>()
            .map(Value::Array),
    }
}

fn ordinate(fn_name: &str, v: &Value) -> Result<f64, Error> {
    v.as_f64()
        .filter(|f| f.is_finite())
        .ok_or_else(|| Error::Validation(format!("{fn_name}: {v} is not a finite ordinate")))
}

/// Wrap a projected ordinate, erroring rather than degrading it.
///
/// `serde_json::Number::from_f64` returns `None` for a non-finite value, and the
/// tempting `.unwrap_or(Value::Null)` would turn a diverged projection into a
/// silently null ordinate — a structurally valid geometry with a hole in it. The
/// backends already reject non-finite results, so this is a belt-and-braces guard
/// that must stay loud.
fn number(fn_name: &str, v: f64) -> Result<Value, Error> {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .ok_or_else(|| {
            Error::Validation(format!(
                "{fn_name}: projection produced a non-finite ordinate ({v})"
            ))
        })
}

/// Turn an SRID into the `INTEGER` a SQL row carries.
pub(super) fn srid_literal(fn_name: &str, srid: Crs) -> Result<Literal, Error> {
    i32::try_from(srid.srid()).map(Literal::Int).map_err(|_| {
        Error::Validation(format!("{fn_name}: SRID {srid} does not fit in an INTEGER"))
    })
}
