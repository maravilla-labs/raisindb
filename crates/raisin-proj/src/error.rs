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

//! Errors raised while parsing CRS identifiers or reprojecting coordinates.

use thiserror::Error;

/// The set of backends that can satisfy a transform, in ascending order of
/// capability and build cost. Reported in error messages so an operator learns
/// exactly which Cargo feature would make a failing transform work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Always compiled in. Exact closed-form WGS84 <-> WebMercator and UTM.
    Builtin,
    /// `proj4rs-backend`: pure Rust, broad EPSG coverage, no system libraries.
    Proj4rs,
    /// `proj-backend`: C libproj >= 9.6, full EPSG database and datum grids.
    Proj,
}

impl Backend {
    /// The Cargo feature that enables this backend, or `None` for the built-in.
    pub fn feature_name(&self) -> Option<&'static str> {
        match self {
            Backend::Builtin => None,
            Backend::Proj4rs => Some("proj4rs-backend"),
            Backend::Proj => Some("proj-backend"),
        }
    }
}

/// Failure modes of CRS parsing and reprojection.
///
/// Every variant is actionable: an unsupported transform names the feature that
/// would support it rather than silently degrading to untransformed coordinates.
#[derive(Debug, Error)]
pub enum ProjError {
    /// The SRID/CRS string could not be parsed (e.g. not `EPSG:<code>`).
    #[error("unrecognized CRS identifier '{input}': expected 'EPSG:<code>', 'SRID=<code>' or a bare numeric code")]
    UnrecognizedCrs { input: String },

    /// The numeric SRID is not a CRS this build knows how to handle.
    ///
    /// This is the "be loud, never approximate" path: it names the feature that
    /// would add support instead of returning the input unchanged.
    #[error(
        "SRID {srid} is not supported by the '{active}' projection backend. \
         Rebuild raisin-server with --features {suggestion} to enable it"
    )]
    UnsupportedSrid {
        srid: u32,
        active: &'static str,
        suggestion: &'static str,
    },

    /// Both CRS are known but no compiled backend can bridge them.
    #[error(
        "no compiled backend can transform SRID {from} -> SRID {to}. \
         Rebuild raisin-server with --features {suggestion} to enable it"
    )]
    UnsupportedTransform {
        from: u32,
        to: u32,
        suggestion: &'static str,
    },

    /// The coordinate lies outside the valid domain of the target projection.
    #[error("coordinate ({x}, {y}) is outside the valid domain of SRID {srid}: {reason}")]
    OutOfDomain {
        x: f64,
        y: f64,
        srid: u32,
        reason: &'static str,
    },

    /// The backend was invoked but failed internally.
    #[error("projection backend '{backend}' failed transforming SRID {from} -> {to}: {message}")]
    BackendFailure {
        backend: &'static str,
        from: u32,
        to: u32,
        message: String,
    },
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, ProjError>;
