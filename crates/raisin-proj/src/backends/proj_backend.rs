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

//! Full PROJ backend: the complete EPSG database plus datum grid shifts.
//!
//! Enabled by the `proj-backend` feature, which requires **libproj >= 9.6** on
//! the build host (located via `pkg-config`) and on the run host.
//! `proj-backend-bundled` instead compiles libproj from vendored source and
//! additionally needs a C/C++ toolchain, sqlite3 and libtiff headers.
//!
//! # Design notes for maintainers
//!
//! * **`proj::Proj` is neither `Send` nor `Sync`** — it owns raw `PJ_CONTEXT` and
//!   `PJ` pointers. It therefore cannot live in a global cache. The pipeline
//!   cache below is `thread_local!`, which is also what PROJ itself wants: one
//!   context per thread.
//! * **Building a pipeline is expensive** (it hits the PROJ SQLite database), so
//!   caching it per `(from, to)` pair is not an optimisation but a requirement
//!   for a per-row `ST_TRANSFORM`.
//! * **Axis order is already handled.** `Proj::new_known_crs` applies
//!   `proj_normalize_for_visualization`, which forces lon/lat and
//!   easting/northing ordering. That matches RaisinDB's x=longitude convention,
//!   so no manual swap is needed — and adding one would break it. Without that
//!   normalisation, EPSG:4326 would use its authority-defined lat/lon order.

use std::cell::RefCell;
use std::collections::HashMap;

use proj::Proj;

use crate::crs::Crs;
use crate::error::{ProjError, Result};

/// Name used in diagnostics for this backend.
pub const BACKEND_NAME: &str = "proj";

thread_local! {
    /// Per-thread cache of PROJ pipelines, keyed by (from SRID, to SRID).
    ///
    /// `None` records a pair PROJ rejected, so a repeatedly failing query does
    /// not re-hit the PROJ database on every row.
    static PIPELINES: RefCell<HashMap<(u32, u32), Option<Proj>>> =
        RefCell::new(HashMap::new());
}

/// Whether PROJ can resolve this CRS on its own.
///
/// Implemented by asking PROJ to build a pipeline from the code to WGS84, which
/// is the only honest way to answer — PROJ's database, not a hardcoded list, is
/// the authority.
pub fn supports(crs: Crs) -> bool {
    if crs == Crs::WGS84 {
        return true;
    }
    with_pipeline(crs, Crs::WGS84, |p| p.is_some())
}

/// Run `f` with the cached pipeline for this pair, building it on first use.
fn with_pipeline<R>(from: Crs, to: Crs, f: impl FnOnce(Option<&Proj>) -> R) -> R {
    PIPELINES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = cache
            .entry((from.srid(), to.srid()))
            .or_insert_with(|| Proj::new_known_crs(&from.to_string(), &to.to_string(), None).ok());
        f(entry.as_ref())
    })
}

/// Transform one coordinate through PROJ.
///
/// Units are whatever the CRS declares — degrees for geographic CRS. Unlike
/// `proj4rs`, PROJ takes degrees for EPSG:4326, so no radian conversion happens
/// here.
pub fn transform(from: Crs, to: Crs, x: f64, y: f64) -> Result<(f64, f64)> {
    with_pipeline(from, to, |pipeline| {
        let pipeline = pipeline.ok_or(ProjError::UnsupportedTransform {
            from: from.srid(),
            to: to.srid(),
            suggestion: "proj-backend-bundled, or a libproj build with a fuller EPSG database",
        })?;

        let (out_x, out_y) = pipeline
            .convert((x, y))
            .map_err(|e| ProjError::BackendFailure {
                backend: BACKEND_NAME,
                from: from.srid(),
                to: to.srid(),
                message: e.to_string(),
            })?;

        // PROJ signals "no image" with HUGE_VAL / inf rather than an error in
        // some pipelines. Never let that reach the index.
        if !out_x.is_finite() || !out_y.is_finite() {
            return Err(ProjError::OutOfDomain {
                x,
                y,
                srid: to.srid(),
                reason:
                    "PROJ produced a non-finite coordinate (point outside the projection domain)",
            });
        }
        Ok((out_x, out_y))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrees_with_builtin_on_web_mercator() {
        let ours = crate::builtin::mercator::wgs84_to_web_mercator(8.54, 47.37).unwrap();
        let theirs = transform(Crs::WGS84, Crs::WEB_MERCATOR, 8.54, 47.37).unwrap();
        assert!((ours.0 - theirs.0).abs() < 1e-3, "{ours:?} vs {theirs:?}");
        assert!((ours.1 - theirs.1).abs() < 1e-3, "{ours:?} vs {theirs:?}");
    }

    #[test]
    fn agrees_with_builtin_on_utm() {
        let ours = crate::builtin::utm::wgs84_to_utm(8.54, 47.37, 32, true, 32632).unwrap();
        let theirs = transform(Crs::WGS84, Crs::from_srid(32632), 8.54, 47.37).unwrap();
        assert!((ours.0 - theirs.0).abs() < 0.01, "{ours:?} vs {theirs:?}");
        assert!((ours.1 - theirs.1).abs() < 0.01, "{ours:?} vs {theirs:?}");
    }

    #[test]
    fn axis_order_is_lon_lat_not_lat_lon() {
        // The canonical PROJ trap: EPSG:4326's authority axis order is lat/lon.
        // `new_known_crs` normalises it, so passing (lon, lat) must be correct.
        // If normalisation ever regressed, easting would come back near the
        // value for longitude 47 instead of longitude 8.54.
        let (x, _) = transform(Crs::WGS84, Crs::WEB_MERCATOR, 8.54, 47.37).unwrap();
        assert!(
            (x - 950_668.45).abs() < 1.0,
            "easting {x} suggests swapped axes"
        );
    }

    #[test]
    fn caches_pipelines_per_pair() {
        // Second call must hit the cache; correctness is all we can assert here.
        let a = transform(Crs::WGS84, Crs::WEB_MERCATOR, 1.0, 2.0).unwrap();
        let b = transform(Crs::WGS84, Crs::WEB_MERCATOR, 1.0, 2.0).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn covers_a_crs_the_builtin_cannot() {
        assert!(supports(Crs::from_srid(31_370)));
        let (x, y) = transform(Crs::WGS84, Crs::from_srid(31_370), 4.35, 50.85).unwrap();
        assert!((140_000.0..160_000.0).contains(&x), "easting {x}");
        assert!((160_000.0..180_000.0).contains(&y), "northing {y}");
    }
}
