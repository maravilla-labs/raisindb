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

//! ST_SETSRID — relabel a geometry's CRS **without moving it**.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::crs::{geometry_type_of, srid_arg};
use super::z_support::{expect_arity, geometry_arg};

/// Assign an SRID to a geometry, leaving every coordinate exactly as it was.
///
/// # SQL Signature
/// `ST_SETSRID(geometry, srid) -> GEOMETRY`
///
/// # ST_SETSRID vs ST_TRANSFORM — the distinction that matters most
///
/// **`ST_SETSRID` reinterprets. `ST_TRANSFORM` moves.**
///
/// ```sql
/// -- The data is Swiss LV95 easting/northing but arrived unlabelled.
/// -- Fix the LABEL. Coordinates are already correct.
/// UPDATE 'sites' SET geom = ST_SETSRID(geom, 2056);
///
/// -- The data really is WGS84 and you want it in LV95.
/// -- CONVERT it. Coordinates change.
/// SELECT ST_TRANSFORM(geom, 2056) FROM 'sites';
/// ```
///
/// Reaching for `ST_SETSRID` where `ST_TRANSFORM` was meant produces a geometry
/// that *claims* to be in the target CRS while its numbers still describe the
/// source one — a silently wrong dataset with no error anywhere. It is the most
/// common multi-CRS mistake, so: if the numbers should change, you want
/// `ST_TRANSFORM`.
///
/// # Semantics
///
/// * Any positive EPSG code is accepted, including codes this build cannot
///   *transform*. A label is a statement of fact about the data; whether a
///   conversion is available is a separate question, answered by `ST_TRANSFORM`.
///   (The spatial index is stricter — see `raisin_proj::normalize_for_index`.)
/// * `ST_SETSRID(g, 4326)` **removes** the `srid` member rather than writing
///   `4326`, keeping the output strictly RFC 7946 conformant.
/// * Deprecated WebMercator synonyms are canonicalised: `ST_SETSRID(g, 900913)`
///   labels the geometry `3857`.
/// * A textual CRS is accepted (`ST_SETSRID(g, 'EPSG:2056')`); a foreign authority
///   is not, because `ESRI:102100` is not `EPSG:102100`.
pub struct StSetSridFunction;

impl SqlFunction for StSetSridFunction {
    fn name(&self) -> &str {
        "ST_SETSRID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_SETSRID(geometry, srid) -> GEOMETRY"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_SETSRID", self.signature(), args, 2)?;
        let Some(geom) = geometry_arg("ST_SETSRID", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(srid) = srid_arg("ST_SETSRID", args, 1, row)? else {
            return Ok(Literal::Null);
        };

        // Validate the shape before stamping: labelling a non-geometry would
        // produce a value that looks CRS-aware and is not.
        geometry_type_of("ST_SETSRID", &geom)?;

        Ok(Literal::Geometry(raisin_geometry::with_srid(geom, srid)))
    }
}
