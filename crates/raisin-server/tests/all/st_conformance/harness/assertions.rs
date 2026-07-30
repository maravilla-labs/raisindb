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

//! The assertion primitives on [`Ctx`].
//!
//! Split out of the setup half purely for size. Every one records coverage from
//! the SQL it ran and pushes into `failures` rather than panicking — see the
//! field docs on `Ctx::failures` for why the suite accumulates instead of
//! aborting.

use serde_json::Value;

use super::Ctx;

impl Ctx {
    // ---- assertion primitives -------------------------------------------
    //
    // Every one records coverage from the SQL it ran and pushes a message into
    // `failures` rather than panicking.

    fn fail(&mut self, what: &str, msg: String) {
        println!("  [FAIL] {what}: {msg}");
        self.failures.push(format!("{what}: {msg}"));
    }

    fn pass(&mut self, what: &str) {
        println!("  [ ok ] {what}");
    }

    /// Assert a scalar expression equals `expected` exactly (JSON equality).
    pub async fn eq(&mut self, what: &str, expr: &str, expected: Value) {
        self.cov.record_sql(expr, what);
        match self.scalar(expr).await {
            Ok(got) if got == expected => self.pass(what),
            Ok(got) => self.fail(what, format!("expected {expected}, got {got}  [{expr}]")),
            Err(e) => self.fail(what, format!("error: {e}  [{expr}]")),
        }
    }

    /// Assert a scalar boolean.
    pub async fn is_true(&mut self, what: &str, expr: &str) {
        self.eq(what, expr, Value::Bool(true)).await
    }

    pub async fn is_false(&mut self, what: &str, expr: &str) {
        self.eq(what, expr, Value::Bool(false)).await
    }

    /// Assert a scalar is SQL NULL.
    pub async fn is_null(&mut self, what: &str, expr: &str) {
        self.eq(what, expr, Value::Null).await
    }

    /// Assert a numeric scalar is within `tol` of `expected` (absolute).
    pub async fn near(&mut self, what: &str, expr: &str, expected: f64, tol: f64) {
        self.cov.record_sql(expr, what);
        match self.scalar(expr).await {
            Ok(v) => match v.as_f64() {
                Some(got) if (got - expected).abs() <= tol => {
                    println!("  [ ok ] {what}  ({got:.6} ~= {expected:.6} +/- {tol})");
                }
                Some(got) => self.fail(
                    what,
                    format!("expected {expected} +/- {tol}, got {got}  [{expr}]"),
                ),
                None => self.fail(what, format!("expected a number, got {v}  [{expr}]")),
            },
            Err(e) => self.fail(what, format!("error: {e}  [{expr}]")),
        }
    }

    /// Assert a numeric scalar is within `rel` *relative* tolerance.
    ///
    /// Used where the reference value is itself approximate (a projected
    /// distance, a polygonal buffer approximating a circle).
    pub async fn near_rel(&mut self, what: &str, expr: &str, expected: f64, rel: f64) {
        let tol = expected.abs() * rel;
        self.near(what, expr, expected, tol).await
    }

    /// Assert a predicate holds on the numeric result, described by `desc`.
    pub async fn num_matches(
        &mut self,
        what: &str,
        expr: &str,
        desc: &str,
        pred: impl Fn(f64) -> bool,
    ) {
        self.cov.record_sql(expr, what);
        match self.scalar(expr).await {
            Ok(v) => match v.as_f64() {
                Some(got) if pred(got) => println!("  [ ok ] {what}  ({got} {desc})"),
                Some(got) => self.fail(what, format!("{got} is not {desc}  [{expr}]")),
                None => self.fail(what, format!("expected a number, got {v}  [{expr}]")),
            },
            Err(e) => self.fail(what, format!("error: {e}  [{expr}]")),
        }
    }

    /// Assert the expression FAILS, and that the message mentions `needle`.
    ///
    /// "Errors loudly" is a behaviour under test in its own right — a silently
    /// wrong answer is the failure mode this whole pass exists to remove, so
    /// "did not error" must be a test failure where an error is the contract.
    pub async fn errors_with(&mut self, what: &str, expr: &str, needle: &str) {
        self.cov.record_sql(expr, what);
        match self.scalar(expr).await {
            Ok(v) => self.fail(
                what,
                format!("expected an error mentioning {needle:?}, got {v}  [{expr}]"),
            ),
            Err(e)
                if e.to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase()) =>
            {
                self.pass(what)
            }
            Err(e) => self.fail(
                what,
                format!("expected an error mentioning {needle:?}, got: {e}"),
            ),
        }
    }

    /// Assert a whole-query result set equals `expected` (list of row maps).
    pub async fn rows_eq(&mut self, what: &str, sql: &str, expected: Vec<Value>) {
        self.cov.record_sql(sql, what);
        match self.sql(sql).await {
            Ok(got) if got == expected => self.pass(what),
            Ok(got) => self.fail(
                what,
                format!(
                    "expected {}, got {}",
                    Value::Array(expected),
                    Value::Array(got)
                ),
            ),
            Err(e) => self.fail(what, format!("error: {e}  [{sql}]")),
        }
    }

    /// Record coverage for a function exercised indirectly (no direct SQL of its
    /// own), with an explicit note.
    pub fn note(&mut self, func: &str, what: &str) {
        self.cov.record(func, what);
    }

    /// Record a defect outside the `ST_*` library. Printed loudly, reported, but
    /// does not fail the run — see the field docs on `product_gaps`.
    pub fn gap(&mut self, msg: String) {
        println!("  [PRODUCT GAP] {msg}");
        self.product_gaps.push(msg);
    }
}
