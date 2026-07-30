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

//! ST_TRANSFORM — reproject a geometry into another CRS.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::crs::{effective_srid, reproject_geometry, srid_arg};
use super::z_support::{expect_arity, geometry_arg};

/// Reproject a geometry from its current CRS into `srid`.
///
/// # SQL Signature
/// `ST_TRANSFORM(geometry, srid) -> GEOMETRY`
///
/// # ST_TRANSFORM vs ST_SETSRID
///
/// **`ST_TRANSFORM` moves the geometry. `ST_SETSRID` only relabels it.** If the
/// coordinates should keep their values and only the declared CRS was wrong, you
/// want `ST_SETSRID`; if the coordinates should be converted, you want this.
///
/// # Axis order
///
/// `(x, y)` is `(longitude, latitude)` for geographic CRSs and
/// `(easting, northing)` for projected ones, in both input and output, for every
/// EPSG code. This deliberately diverges from the EPSG authority, which defines
/// EPSG:4326 as `(latitude, longitude)`: GeoJSON RFC 7946 §3.1.1, PostGIS,
/// `geo-types` and every web mapping library are lon/lat, and honouring authority
/// order would break interop with all of them and with our own stored data.
/// The OGC URN form does **not** flip the axes:
/// `ST_TRANSFORM(g, 'urn:ogc:def:crs:EPSG::4326')` means the same as
/// `ST_TRANSFORM(g, 4326)`. There is no per-code axis flipping anywhere.
///
/// # Guaranteed coverage, and what needs a Cargo feature
///
/// A default build — no system libraries, no C toolchain — handles EPSG:4326,
/// EPSG:3857 (with its 3785/900913 aliases) and all 120 WGS84 UTM zones exactly.
/// Wider coverage is opt-in: `--features proj` adds roughly a thousand EPSG codes
/// in pure Rust, `--features proj-full` the whole EPSG database plus datum grids.
///
/// # There is no silent fallback
///
/// When no compiled backend can perform the requested pair, this **errors**, and
/// the message names both codes and the Cargo feature that would enable them.
/// Returning the input unprojected, or projecting approximately, would produce a
/// geometry that is wrong by hundreds of kilometres with nothing to indicate it.
/// The same applies to a coordinate with no image in the target CRS — a pole
/// against EPSG:3857, for instance, where libproj happily returns a *finite*
/// northing twelve times the height of the Mercator world.
///
/// # Altitude and emptiness
///
/// Every transform available here is horizontal, so a third (and any fourth)
/// ordinate rides through untouched: `ST_TRANSFORM` preserves altitude, unlike the
/// 2-D `geo` pipeline the measurement functions use. An empty geometry transforms
/// to itself. A `Feature` is reduced to its geometry, matching how the rest of the
/// ST_\* family treats one, and loses altitude on that path.
///
/// # All or nothing
///
/// A single out-of-domain coordinate fails the whole geometry. Emitting a
/// partially projected ring would yield a structurally valid polygon describing
/// nowhere in particular, which is worse than an error.
pub struct StTransformFunction;

impl SqlFunction for StTransformFunction {
    fn name(&self) -> &str {
        "ST_TRANSFORM"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_TRANSFORM(geometry, srid) -> GEOMETRY"
    }

    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_TRANSFORM", self.signature(), args, 2)?;
        let Some(geom) = geometry_arg("ST_TRANSFORM", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(to) = srid_arg("ST_TRANSFORM", args, 1, row)? else {
            return Ok(Literal::Null);
        };

        let from = effective_srid("ST_TRANSFORM", &geom)?;
        if from == to {
            // Identity, but still stamp the label so that transforming an
            // unlabelled geometry to a non-4326 CRS it already sits in produces a
            // *labelled* result. Cheaper than walking the coordinates, and it
            // means `ST_TRANSFORM(g, 900913)` on 3857 data is a relabel, not an
            // unsupported pair.
            return Ok(Literal::Geometry(raisin_geometry::with_srid(geom, to)));
        }

        Ok(Literal::Geometry(reproject_geometry(
            "ST_TRANSFORM",
            &geom,
            from,
            to,
        )?))
    }
}
