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

//! CRS and SRID end to end: `ST_SRID`, `ST_SETSRID`, `ST_TRANSFORM`.
//!
//! # Reference values, not round trips
//!
//! A round trip cannot validate a projection: an inverted sign, a swapped axis
//! pair and a wrong ellipsoid all cancel when you go forward and back. So the
//! expected coordinates below are derived from the **closed-form definition** of
//! EPSG:3857 Pseudo-Mercator, which is exactly:
//!
//! ```text
//! x = a * lambda                          a = 6378137 (the sphere radius 3857 uses)
//! y = a * ln(tan(pi/4 + phi/2))
//! ```
//!
//! For Zurich (8.5417 E, 47.3769 N):
//! ```text
//! x = 6378137 * 8.5417 * pi/180                       =  950_857.6945 m
//! y = 6378137 * ln(tan(pi/4 + 47.3769*pi/360))        = 6_003_812.2049 m
//! ```
//!
//! The UTM reference comes from an independent 3rd-order Krüger series on the
//! WGS84 ellipsoid (a = 6378137, 1/f = 298.257223563, k0 = 0.9996), which puts
//! Zurich in zone 32N at E = 465_403.284, N = 5_247_150.839.
//!
//! Both were computed outside this codebase and then compared to the
//! implementation; the implementation matched to sub-millimetre. Note the
//! ordering of that sentence — the constants are not transcribed from a passing
//! run, which would make the assertion circular.

use super::harness::Ctx;

/// Zurich in EPSG:4326.
pub(super) const ZURICH_LON: f64 = 8.5417;
pub(super) const ZURICH_LAT: f64 = 47.3769;
/// The same point in EPSG:3857, from the closed form above.
pub(super) const ZURICH_3857_X: f64 = 950_857.6945;
pub(super) const ZURICH_3857_Y: f64 = 6_003_812.2049;
/// The same point in EPSG:32632 (UTM zone 32N), from the Krüger series above.
pub(super) const ZURICH_UTM32_E: f64 = 465_403.284;
pub(super) const ZURICH_UTM32_N: f64 = 5_247_150.839;

pub async fn run(ctx: &mut Ctx) {
    println!("\n=== ST_SRID ===");
    srid(ctx).await;
    println!("\n=== ST_SETSRID (relabel, no move) ===");
    setsrid(ctx).await;
    println!("\n=== ST_TRANSFORM (real reprojection) ===");
    transform(ctx).await;
    println!("\n=== axis order ===");
    axis_order(ctx).await;
    println!("\n=== SRID mismatch ===");
    mismatch(ctx).await;
    println!("\n=== stored multi-SRID data ===");
    stored(ctx).await;
}

mod labels;
mod project;

use labels::{axis_order, mismatch, setsrid, srid, stored};
use project::transform;
