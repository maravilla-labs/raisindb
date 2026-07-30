//! Shared argument plumbing for the three-dimensional ST_* functions.
//!
//! # The 3-D model, stated once
//!
//! `geo_types::Coord` is strictly two dimensional — there is no Z anywhere in the
//! `geo` pipeline. Altitude is therefore read off the **GeoJSON representation**,
//! via `raisin_geometry::zdim`, and never off a `geo::Geometry`. A per-vertex Z
//! vector could not survive `BooleanOps`, `Buffer` or `ConvexHull`, all of which
//! invent new vertices, so the most that is kept is the `(min, max)` interval.
//!
//! **Every other ST_\* function ignores Z**, exactly as PostGIS's 2-D predicates
//! do. That is a deliberate match with PostGIS, not a shortfall, and it is stated
//! here rather than repeated as a caveat on forty functions.

use serde_json::Value;

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Evaluate one argument as a GeoJSON value.
///
/// `Ok(None)` means the argument evaluated to SQL NULL, which every ST_\*
/// function propagates as NULL rather than treating as an error.
///
/// `Text` is accepted because `ST_GEOMFROMGEOJSON` is frequently skipped and a
/// raw JSON string passed instead; rejecting it would be a gratuitous papercut.
///
/// # Nested property paths
///
/// When the argument is `properties->>'venue.geo'` and the ordinary JSON lookup
/// finds no such key, the dotted path is resolved against the row's property tree
/// (see [`super::property_path`]). The direct key wins, so this is a fallback and
/// never an override.
///
/// A **wildcard** path (`stops[].geo`) names a set, which has no single value.
/// It is rejected here with a message naming the two functions that define an
/// answer over a set — `ST_DWITHIN` (any) and `ST_DISTANCE` (minimum) — rather
/// than silently picking one element, which would be a wrong answer for
/// `ST_INTERSECTS` and friends.
pub(super) fn geometry_arg(
    fn_name: &str,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<Value>, Error> {
    if let Some(value) = geometry_arg_direct(fn_name, args, index, row)? {
        return Ok(Some(value));
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

/// [`geometry_arg`] WITHOUT the nested-path fallback.
///
/// Exists so `convert::geom_pair_multi` can tell "the direct JSON key answered"
/// from "the path walk still has to run" without re-running the fallback and
/// without swallowing a genuine type error.
pub(super) fn geometry_arg_direct(
    fn_name: &str,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<Value>, Error> {
    match eval_expr(&args[index], row)? {
        Literal::Null => Ok(None),
        Literal::Geometry(v) | Literal::JsonB(v) => Ok(Some(v)),
        Literal::Text(s) => serde_json::from_str(&s).map(Some).map_err(|e| {
            Error::Validation(format!(
                "{fn_name}: argument {} is not valid GeoJSON: {e}",
                index + 1
            ))
        }),
        other => Err(Error::Validation(format!(
            "{fn_name}: argument {} must be a GEOMETRY, got {:?}",
            index + 1,
            other.data_type()
        ))),
    }
}

/// Evaluate one argument as a number, propagating NULL.
pub(super) fn numeric_arg(
    fn_name: &str,
    args: &[TypedExpr],
    index: usize,
    row: &Row,
) -> Result<Option<f64>, Error> {
    match eval_expr(&args[index], row)? {
        Literal::Null => Ok(None),
        Literal::Double(d) => Ok(Some(d)),
        Literal::Int(i) => Ok(Some(i as f64)),
        Literal::BigInt(i) => Ok(Some(i as f64)),
        other => Err(Error::Validation(format!(
            "{fn_name}: argument {} must be numeric, got {:?}",
            index + 1,
            other.data_type()
        ))),
    }
}

/// Reject a wrong argument count with a message naming the signature.
pub(super) fn expect_arity(
    fn_name: &str,
    signature: &str,
    args: &[TypedExpr],
    n: usize,
) -> Result<(), Error> {
    if args.len() != n {
        return Err(Error::Validation(format!(
            "{fn_name} requires exactly {n} argument(s): {signature}"
        )));
    }
    Ok(())
}
