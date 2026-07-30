//! The **single** spatial predicate extractor.
//!
//! Historically this logic existed twice, byte-for-byte: once in
//! `hierarchy_rewrite::rewrite::rewrite_st_dwithin` and once in the execution
//! planner's `filter_analysis::match_canonical_predicate`. Two copies of a
//! pattern matcher for the same syntax is how one of them ends up recognising a
//! shape the other does not, which in a spatial planner means "the index is used
//! here and not there" — and, before the fail-closed catalog landed, that meant
//! two different answers to the same query. Both call sites now delegate here.
//!
//! # What is recognised
//!
//! * `ST_DWITHIN(<geom source>, <const geometry>, <const radius>)` in **either**
//!   argument order.
//! * `ST_DISTANCE(<geom source>, <const geometry>) < | <= <const radius>` and
//!   its reversed spelling, which denote the same access path.
//! * A centre that is any constant-foldable geometry: `ST_POINT`/`ST_MAKEPOINT`
//!   of numeric literals, `ST_GEOMFROMGEOJSON('...')`, `ST_SETSRID(<const>, n)`,
//!   a `Literal::Geometry`, a `Literal::JsonB` holding GeoJSON, or a text
//!   literal parseable as GeoJSON. A **non-point** centre is reduced to its
//!   envelope centre with the radius inflated to the envelope circumradius, and
//!   the result is then marked inexact.
//!
//! # The exactness contract
//!
//! [`CanonicalPredicate::SpatialDWithin::exact`] is the planner's licence to drop
//! the predicate from the residual filter. It is `true` only when a *complete*
//! index scan over `(center_lon, center_lat, radius_meters)` returns precisely
//! the predicate's match set. Every widening in this module — reversed operands
//! aside, which changes nothing — sets it to `false` and keeps the verbatim
//! source expression in `original`, so the widened scan can only ever supply
//! candidates.

mod consts;
mod srid_gate;
#[cfg(test)]
mod srid_tests;

pub use consts::{
    const_geometry, const_geometry_srid, envelope_center, envelope_circumradius_meters,
    geojson_envelope, numeric_literal,
};

use super::predicate::CanonicalPredicate;
use crate::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};
use serde_json::Value;
use srid_gate::query_center_is_wgs84;

/// Extract the geometry source `(table, geometry_column, property_name)` from an
/// expression.
///
/// Handles direct property access and `CAST(... AS GEOMETRY)` wrappers:
/// - `properties->>'location'` (JsonExtractText)
/// - `properties->'location'` (JsonExtract)
/// - `CAST(properties->>'location' AS GEOMETRY)`
/// - a bare column reference, which is read as `properties->>'<column>'`
pub fn extract_geometry_source(expr: &Expr) -> Option<(String, String, String)> {
    match expr {
        // Unwrap CAST(... AS GEOMETRY) only — reject other target types.
        Expr::Cast {
            expr: inner,
            target_type: crate::analyzer::DataType::Geometry,
        } => extract_geometry_source(&inner.expr),

        Expr::JsonExtractText { object, key } | Expr::JsonExtract { object, key } => {
            if let Expr::Column { table, column } = &object.expr {
                let prop = match &key.expr {
                    Expr::Literal(Literal::Text(t)) => t.clone(),
                    _ => return None,
                };
                Some((table.clone(), column.clone(), prop))
            } else {
                None
            }
        }
        Expr::Column { table, column } => {
            Some((table.clone(), "properties".to_string(), column.clone()))
        }
        _ => None,
    }
}

/// Recognise a spatial predicate. Returns `None` when `expr` is not one of the
/// index-eligible spatial shapes; the caller then keeps it as `Other`.
pub fn extract_spatial_predicate(expr: &TypedExpr) -> Option<CanonicalPredicate> {
    match &expr.expr {
        Expr::Function { name, args, .. } if name.eq_ignore_ascii_case("ST_DWITHIN") => {
            match_dwithin(args, expr)
        }
        // `ST_DISTANCE(g, <const>) < r` / `<= r` is the same access path as
        // ST_DWITHIN. `>` / `>=` are anti-ranges with NO index path and must fall
        // through to a scan with the predicate retained.
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::Lt | BinaryOperator::LtEq => {
                match_distance_bound(left, right, op, expr)
            }
            BinaryOperator::Gt | BinaryOperator::GtEq => {
                // reversed spelling: `r > ST_DISTANCE(...)`
                let flipped = match op {
                    BinaryOperator::Gt => BinaryOperator::Lt,
                    _ => BinaryOperator::LtEq,
                };
                match_distance_bound(right, left, &flipped, expr)
            }
            _ => None,
        },
        _ => None,
    }
}

/// `ST_DWITHIN(a, b, radius)` where exactly one of `a`/`b` is a geometry source
/// and the other is a constant geometry.
fn match_dwithin(args: &[TypedExpr], original: &TypedExpr) -> Option<CanonicalPredicate> {
    if args.len() != 3 {
        return None;
    }
    let radius = numeric_literal(&args[2])?;
    if !radius.is_finite() || radius < 0.0 {
        return None;
    }

    // Either argument order. A constant on BOTH sides is not a scan predicate at
    // all (it is a constant expression), and a source on both sides is a join.
    let (source, center) = match (
        extract_geometry_source(&args[0].expr),
        const_geometry(&args[1]),
        extract_geometry_source(&args[1].expr),
        const_geometry(&args[0]),
    ) {
        (Some(src), Some(c), _, None) => (src, c),
        (_, None, Some(src), Some(c)) => (src, c),
        // Ambiguous (both sides look like a source) — prefer the conventional
        // left-source / right-constant reading when it is available.
        (Some(src), Some(c), Some(_), _) => (src, c),
        _ => {
            tracing::debug!(
                "ST_DWITHIN not index-eligible: needs one geometry source and one \
                 constant geometry argument"
            );
            return None;
        }
    };

    build_dwithin(source, &center, radius, true, original)
}

/// `ST_DISTANCE(<source>, <const>) <op> <const radius>`.
fn match_distance_bound(
    distance_side: &TypedExpr,
    bound_side: &TypedExpr,
    op: &BinaryOperator,
    original: &TypedExpr,
) -> Option<CanonicalPredicate> {
    let Expr::Function { name, args, .. } = &distance_side.expr else {
        return None;
    };
    if !name.eq_ignore_ascii_case("ST_DISTANCE") || args.len() != 2 {
        return None;
    }
    let radius = numeric_literal(bound_side)?;
    if !radius.is_finite() || radius < 0.0 {
        return None;
    }

    let (source, center) = match (
        extract_geometry_source(&args[0].expr),
        const_geometry(&args[1]),
        extract_geometry_source(&args[1].expr),
        const_geometry(&args[0]),
    ) {
        (Some(src), Some(c), _, None) => (src, c),
        (_, None, Some(src), Some(c)) => (src, c),
        (Some(src), Some(c), Some(_), _) => (src, c),
        _ => return None,
    };

    // `ST_DISTANCE < r` is STRICT while the index scan's own post-filter is
    // `<= r`, so the scan is a superset by exactly the boundary ring. `<= r`
    // matches the scan exactly, but only for a point centre — `build_dwithin`
    // downgrades a non-point centre regardless.
    let exact = matches!(op, BinaryOperator::LtEq);
    build_dwithin(source, &center, radius, exact, original)
}

/// Assemble a `SpatialDWithin` from a geometry source and a constant centre.
///
/// A non-point centre is reduced to its envelope centre with the radius inflated
/// by the envelope circumradius. That is a strict widening (every point within
/// `radius` of the real geometry is within `radius + circumradius` of the
/// envelope centre), so the scan stays a superset — and the result is marked
/// inexact so the original predicate is re-applied per row.
fn build_dwithin(
    source: (String, String, String),
    center: &Value,
    radius: f64,
    exact_when_point: bool,
    original: &TypedExpr,
) -> Option<CanonicalPredicate> {
    let (table, geometry_column, property_name) = source;
    if !query_center_is_wgs84(center) {
        tracing::debug!(
            srid = const_geometry_srid(center),
            "spatial predicate not index-eligible: the constant centre is in a projected CRS; \
             keeping it as a residual filter"
        );
        return None;
    }
    let envelope = geojson_envelope(center)?;
    let (center_lon, center_lat) = envelope_center(&envelope);
    let inflate = envelope_circumradius_meters(&envelope);

    // A degenerate envelope (a single position) IS a point, whatever the
    // GeoJSON type label says.
    let is_point = inflate <= 0.0;
    let exact = is_point && exact_when_point;

    Some(CanonicalPredicate::SpatialDWithin {
        table,
        geometry_column,
        property_name,
        center_lon,
        center_lat,
        radius_meters: radius + inflate,
        exact,
        original: Box::new(original.clone()),
    })
}

/// An `ORDER BY ST_DISTANCE(<source>, <const>)` ordering lifted out of a sort
/// expression.
///
/// This is *not* a [`CanonicalPredicate`]: every element of a canonical
/// predicate list becomes part of the row-level residual filter, and an ordering
/// has no truth value to contribute. Keeping it separate is what stops it from
/// ever being AND-ed into a WHERE clause.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialDistanceOrder {
    pub table: String,
    pub geometry_column: String,
    pub property_name: String,
    pub center_lon: f64,
    pub center_lat: f64,
    /// `true` when the ordering is by the true distance to a POINT centre, so
    /// the index's distance order is exactly the requested order. `false` for a
    /// non-point centre, where centre-of-envelope distance is only an
    /// approximation and the `Sort` must be kept.
    pub exact: bool,
}

/// Recognise `ST_DISTANCE(<geom source>, <const geometry>)` as an ordering key.
pub fn extract_distance_order(expr: &TypedExpr) -> Option<SpatialDistanceOrder> {
    let Expr::Function { name, args, .. } = &expr.expr else {
        return None;
    };
    if !name.eq_ignore_ascii_case("ST_DISTANCE") || args.len() != 2 {
        return None;
    }
    let (source, center) = match (
        extract_geometry_source(&args[0].expr),
        const_geometry(&args[1]),
        extract_geometry_source(&args[1].expr),
        const_geometry(&args[0]),
    ) {
        (Some(src), Some(c), _, None) => (src, c),
        (_, None, Some(src), Some(c)) => (src, c),
        (Some(src), Some(c), Some(_), _) => (src, c),
        _ => return None,
    };
    // Same rule as `build_dwithin`: a projected centre cannot drive the index, and
    // an ORDER BY that silently used ground distance where the row evaluator uses
    // projected distance would order rows differently from the SQL it came from.
    if !query_center_is_wgs84(&center) {
        return None;
    }
    let envelope = geojson_envelope(&center)?;
    let (center_lon, center_lat) = envelope_center(&envelope);
    Some(SpatialDistanceOrder {
        table: source.0,
        geometry_column: source.1,
        property_name: source.2,
        center_lon,
        center_lat,
        exact: envelope_circumradius_meters(&envelope) <= 0.0,
    })
}
