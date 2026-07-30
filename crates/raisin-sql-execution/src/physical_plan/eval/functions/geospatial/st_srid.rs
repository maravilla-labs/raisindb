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

//! ST_SRID — report a geometry's coordinate reference system.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::crs::{effective_srid, geometry_type_of, srid_literal};
use super::z_support::{expect_arity, geometry_arg};

/// Return the SRID (spatial reference identifier) of a geometry.
///
/// # SQL Signature
/// `ST_SRID(geometry) -> INTEGER`
///
/// # This used to be a lie
///
/// It returned the constant `4326` for every input, documented as "RaisinDB always
/// uses WGS84". SRID is now real data: it travels in the geometry value's `srid`
/// member, survives function composition, and is what `ST_TRANSFORM` changes.
///
/// # Semantics
///
/// * Purely a metadata read — no reprojection, no coordinate access at all.
/// * An **unlabelled** geometry (no `srid` member) reports **4326**. That is what
///   keeps every pre-existing query and every RFC 7946 dataset working unchanged,
///   and it is also why unlabelled is kept distinct from an explicit `4326`
///   internally: only an unlabelled operand *adopts* the other side's SRID in a
///   binary predicate.
/// * Deprecated synonyms are canonicalised, so a geometry labelled `900913` or
///   `3785` reports `3857`. They denote the same CRS, and reporting the alias would
///   make `ST_SRID(a) = ST_SRID(b)` false for two identical geometries.
///
/// A geometry stored in a workspace whose schema declares a default SRID is
/// indexed under that default, but `ST_SRID` cannot see schema at expression-eval
/// depth and reports 4326 for it — see `crs::effective_srid` for why.
pub struct StSridFunction;

impl SqlFunction for StSridFunction {
    fn name(&self) -> &str {
        "ST_SRID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_SRID(geometry) -> INTEGER"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_SRID", self.signature(), args, 1)?;
        let Some(geom) = geometry_arg("ST_SRID", args, 0, row)? else {
            return Ok(Literal::Null);
        };

        // Reject a non-geometry rather than reporting 4326 for it: "this JSON blob
        // is in WGS84" is a meaningless claim and hides a modelling error.
        geometry_type_of("ST_SRID", &geom)?;

        srid_literal("ST_SRID", effective_srid("ST_SRID", &geom)?)
    }
}
