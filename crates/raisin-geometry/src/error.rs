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

//! Errors raised while converting geometries.
//!
//! `geojson::Error` and `raisin_proj::ProjError` are mapped into
//! [`GeometryError`] **here and nowhere else**, so no other crate has to know
//! either error type. `GeometryError` in turn converts into
//! [`raisin_error::Error`], which is what every SQL function returns.

use thiserror::Error;

/// Failure modes of geometry conversion.
#[derive(Debug, Error)]
pub enum GeometryError {
    /// The value is not a GeoJSON geometry at all.
    #[error("not a GeoJSON geometry: {reason}")]
    NotGeometry { reason: String },

    /// The value parsed as GeoJSON but could not become a `geo` geometry.
    #[error("cannot convert GeoJSON {geometry_type} to a geo geometry: {reason}")]
    Unconvertible {
        geometry_type: String,
        reason: String,
    },

    /// A coordinate is NaN or infinite.
    ///
    /// Rejected rather than propagated: a non-finite ordinate silently poisons
    /// every `geo` algorithm downstream, and would geohash to a garbage index
    /// cell.
    #[error("geometry has a non-finite coordinate ({x}, {y})")]
    NonFiniteCoordinate { x: f64, y: f64 },

    /// A binary operation was given two geometries in different CRSs.
    ///
    /// Deliberately an error, like PostGIS. An implicit transform would both
    /// hide a data-modelling mistake and make the query's success depend on
    /// which Cargo features the server was built with.
    #[error("{function}: SRID mismatch ({left} vs {right}); wrap one side in ST_TRANSFORM")]
    SridMismatch {
        function: String,
        left: u32,
        right: u32,
    },

    /// The `srid` member is present but not a usable EPSG code.
    #[error("invalid srid member: {reason}")]
    InvalidSrid { reason: String },

    /// A `geo` geometry type RaisinDB's GeoJSON model cannot represent.
    #[error("geo geometry {geometry_type} has no GeoJSON representation")]
    Unrepresentable { geometry_type: &'static str },

    /// Reprojection failed. The message is `ProjError`'s verbatim, which names
    /// the Cargo feature that would make it work.
    #[error(transparent)]
    Projection(#[from] raisin_proj::ProjError),

    /// The value could not be (de)serialized.
    #[error("geometry serialization failed: {0}")]
    Serde(String),
}

impl From<geojson::Error> for GeometryError {
    fn from(e: geojson::Error) -> Self {
        // geojson's own variants already carry the geometry type and the
        // dimension mismatch, so the message is worth keeping verbatim.
        GeometryError::NotGeometry {
            reason: e.to_string(),
        }
    }
}

impl From<serde_json::Error> for GeometryError {
    fn from(e: serde_json::Error) -> Self {
        GeometryError::Serde(e.to_string())
    }
}

impl From<GeometryError> for raisin_error::Error {
    /// Every geometry failure is a bad *input*, not a backend fault, so these
    /// map to `Validation` — which surfaces to a SQL client as a query error
    /// rather than a 500.
    fn from(e: GeometryError) -> Self {
        raisin_error::Error::Validation(e.to_string())
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, GeometryError>;
