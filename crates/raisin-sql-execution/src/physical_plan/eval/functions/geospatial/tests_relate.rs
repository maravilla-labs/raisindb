//! Unit tests for the ten topological predicates and `ST_RELATE`.
//!
//! Split by *what could be wrong* rather than by function:
//!
//! * [`coverage`] — every ordered pair of the seven GeoJSON types must produce a
//!   boolean from every predicate. No "unsupported geometry type" error and no
//!   silent catch-all is acceptable, and the previous implementation had both.
//! * [`identities`] — `DISJOINT == !INTERSECTS`, `WITHIN(a,b) == CONTAINS(b,a)`,
//!   `COVERS`/`COVEREDBY` mirroring, `CONTAINS ⇒ COVERS`, symmetry, reflexivity.
//!   These catch a matrix method wired to the wrong function, which no
//!   single-function test would notice.
//! * [`boundaries`] — where `CONTAINS` and `COVERS` genuinely differ, which is
//!   the whole reason both exist, plus the dimension rules for `CROSSES` and
//!   `OVERLAPS`.
//! * [`shapes`] — interior rings, `Multi*`, nested `GeometryCollection`s, empty
//!   geometries and topological equality: the inputs that used to error.
//! * [`relate_fn`] — `ST_RELATE` itself.
//! * [`plumbing`] — NULL propagation, arity, SRID rules, malformed input.
//!
//! Fixtures and the evaluation harness live here so every submodule shares one
//! set of geometries; a child module sees its ancestors' private items, so
//! `use super::*` is all each needs.

mod boundaries;
mod coverage;
mod identities;
mod plumbing;
mod relate_fn;
mod shapes;

// A descendant module can see its ancestors' private bindings, so each
// submodule's `use super::*` picks up the ten predicate types through this glob
// along with the fixtures below.
use super::*;

use crate::physical_plan::eval::functions::traits::SqlFunction;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::{DataType, Expr, Literal, TypedExpr};
use serde_json::json;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn g(v: serde_json::Value) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Geometry(v)), DataType::Geometry)
}

fn text(s: &str) -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Text(s.to_string())), DataType::Text)
}

fn null() -> TypedExpr {
    TypedExpr::new(Expr::Literal(Literal::Null), DataType::Unknown)
}

/// Evaluate a predicate, requiring a boolean answer.
fn b(f: &dyn SqlFunction, a: serde_json::Value, c: serde_json::Value) -> bool {
    match f.evaluate(&[g(a), g(c)], &Row::new()) {
        Ok(Literal::Boolean(v)) => v,
        other => panic!("{}: expected Boolean, got {other:?}", f.name()),
    }
}

/// The ten predicates, as trait objects, so a test can sweep all of them.
fn all_predicates() -> Vec<Box<dyn SqlFunction>> {
    vec![
        Box::new(StIntersectsFunction),
        Box::new(StDisjointFunction),
        Box::new(StContainsFunction),
        Box::new(StWithinFunction),
        Box::new(StCoversFunction),
        Box::new(StCoveredByFunction),
        Box::new(StTouchesFunction),
        Box::new(StCrossesFunction),
        Box::new(StOverlapsFunction),
        Box::new(StEqualsFunction),
    ]
}

// ---------------------------------------------------------------------------
// Geometry fixtures — one of every GeoJSON type
// ---------------------------------------------------------------------------

fn point() -> serde_json::Value {
    json!({"type":"Point","coordinates":[1.0,1.0]})
}

fn multipoint() -> serde_json::Value {
    json!({"type":"MultiPoint","coordinates":[[1.0,1.0],[8.0,8.0]]})
}

fn line() -> serde_json::Value {
    json!({"type":"LineString","coordinates":[[0.0,0.0],[4.0,4.0]]})
}

fn multiline() -> serde_json::Value {
    json!({"type":"MultiLineString","coordinates":[
        [[0.0,0.0],[4.0,4.0]],
        [[0.0,4.0],[4.0,0.0]]
    ]})
}

/// The unit square scaled to 0..4.
fn square() -> serde_json::Value {
    json!({"type":"Polygon","coordinates":[[
        [0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]
    ]]})
}

fn multipolygon() -> serde_json::Value {
    json!({"type":"MultiPolygon","coordinates":[
        [[[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]]],
        [[[10.0,10.0],[14.0,10.0],[14.0,14.0],[10.0,14.0],[10.0,10.0]]]
    ]})
}

/// A point plus a line, deliberately *disjoint* from each other so that
/// self-relating the collection has an unambiguous answer.
fn collection() -> serde_json::Value {
    json!({"type":"GeometryCollection","geometries":[
        {"type":"Point","coordinates":[8.0,8.0]},
        {"type":"LineString","coordinates":[[0.0,0.0],[4.0,4.0]]}
    ]})
}

fn empty() -> serde_json::Value {
    json!({"type":"GeometryCollection","geometries":[]})
}

/// `square()` with a square hole from 1..3.
fn donut() -> serde_json::Value {
    json!({"type":"Polygon","coordinates":[
        [[0.0,0.0],[4.0,0.0],[4.0,4.0],[0.0,4.0],[0.0,0.0]],
        [[1.0,1.0],[3.0,1.0],[3.0,3.0],[1.0,3.0],[1.0,1.0]]
    ]})
}

fn every_type() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("Point", point()),
        ("MultiPoint", multipoint()),
        ("LineString", line()),
        ("MultiLineString", multiline()),
        ("Polygon", square()),
        ("MultiPolygon", multipolygon()),
        ("GeometryCollection", collection()),
        ("Polygon(with hole)", donut()),
        ("empty", empty()),
    ]
}

fn pt(x: f64, y: f64) -> serde_json::Value {
    json!({"type":"Point","coordinates":[x,y]})
}
