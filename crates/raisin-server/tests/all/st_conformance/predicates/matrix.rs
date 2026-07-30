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

use super::super::fixtures::expr;
use super::super::harness::Ctx;
use super::{ALL_TYPES, PREDICATES};

/// Every predicate against every ordered type pair: 10 x 7 x 7 = 490 calls.
///
/// The assertion is not a specific truth value — it is that the call **succeeds
/// and returns a boolean**. That is precisely the property the old per-type-pair
/// implementation lacked: "unsupported geometry type" errors and silent
/// catch-all `false` were both reachable, and only a sweep finds them.
pub(super) async fn type_matrix(ctx: &mut Ctx) {
    let mut errors = Vec::new();
    let mut calls = 0usize;

    for pred in PREDICATES {
        for a in ALL_TYPES {
            for b in ALL_TYPES {
                let sql = format!("SELECT {pred}({}, {}) AS r", expr(a), expr(b));
                calls += 1;
                match ctx.sql(&sql).await {
                    Ok(rows) => {
                        let v = rows
                            .first()
                            .and_then(|r| r.get("r").cloned())
                            .unwrap_or(serde_json::Value::Null);
                        if !v.is_boolean() {
                            errors.push(format!("{pred}({a}, {b}) returned {v}, not a boolean"));
                        }
                    }
                    Err(e) => errors.push(format!("{pred}({a}, {b}) errored: {e}")),
                }
            }
        }
        ctx.note(pred, "type matrix sweep");
    }

    println!("  swept {calls} predicate/type-pair combinations");
    if errors.is_empty() {
        println!("  [ ok ] every predicate accepts every geometry type pair");
    } else {
        for e in &errors {
            println!("  [FAIL] {e}");
        }
        ctx.failures.push(format!(
            "type matrix: {} of {} combinations failed (first: {})",
            errors.len(),
            calls,
            errors[0]
        ));
    }
}

/// The identities that must hold for every type pair.
///
/// `geo` defines `is_intersects` as `!is_disjoint`, so the first identity is
/// structural now — but it was NOT before: `ST_DISJOINT` covered nine type pairs
/// and `ST_INTERSECTS` six, so `NOT ST_INTERSECTS(a,b)` and `ST_DISJOINT(a,b)`
/// gave different answers on the same rows. Asserting it over the matrix is what
/// keeps that from coming back.
pub(super) async fn identities(ctx: &mut Ctx) {
    let mut violations = Vec::new();
    let mut checks = 0usize;

    for a in ALL_TYPES {
        for b in ALL_TYPES {
            let (ea, eb) = (expr(a), expr(b));
            let cases = [
                (
                    "DISJOINT == NOT INTERSECTS",
                    format!("ST_DISJOINT({ea}, {eb}) = (NOT ST_INTERSECTS({ea}, {eb}))"),
                ),
                (
                    "WITHIN(a,b) == CONTAINS(b,a)",
                    format!("ST_WITHIN({ea}, {eb}) = ST_CONTAINS({eb}, {ea})"),
                ),
                (
                    "COVEREDBY(a,b) == COVERS(b,a)",
                    format!("ST_COVEREDBY({ea}, {eb}) = ST_COVERS({eb}, {ea})"),
                ),
                (
                    "EQUALS is symmetric",
                    format!("ST_EQUALS({ea}, {eb}) = ST_EQUALS({eb}, {ea})"),
                ),
                (
                    "INTERSECTS is symmetric",
                    format!("ST_INTERSECTS({ea}, {eb}) = ST_INTERSECTS({eb}, {ea})"),
                ),
                // Containment implies covering: CONTAINS is strictly stronger.
                (
                    "CONTAINS implies COVERS",
                    format!("(NOT ST_CONTAINS({ea}, {eb})) OR ST_COVERS({ea}, {eb})"),
                ),
                (
                    "WITHIN implies COVEREDBY",
                    format!("(NOT ST_WITHIN({ea}, {eb})) OR ST_COVEREDBY({ea}, {eb})"),
                ),
                // Equality implies mutual covering.
                (
                    "EQUALS implies COVERS both ways",
                    format!(
                        "(NOT ST_EQUALS({ea}, {eb})) OR (ST_COVERS({ea}, {eb}) AND ST_COVERS({eb}, {ea}))"
                    ),
                ),
            ];

            for (name, expr_sql) in cases {
                checks += 1;
                match ctx.scalar(&expr_sql).await {
                    Ok(serde_json::Value::Bool(true)) => {}
                    Ok(other) => {
                        violations.push(format!("{name} failed for ({a}, {b}): got {other}"))
                    }
                    Err(e) => violations.push(format!("{name} errored for ({a}, {b}): {e}")),
                }
            }
        }
    }

    println!("  checked {checks} identity instances over the type matrix");
    if violations.is_empty() {
        println!("  [ ok ] every algebraic identity holds for every type pair");
    } else {
        for v in violations.iter().take(20) {
            println!("  [FAIL] {v}");
        }
        ctx.failures.push(format!(
            "identities: {} of {} instances violated (first: {})",
            violations.len(),
            checks,
            violations[0]
        ));
    }
}
