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

//! Measurement: area, length, perimeter, distance, azimuth.
//!
//! # Where the expected numbers come from
//!
//! Not from running the implementation. Every reference value below is derived
//! from a published constant or a closed form, because a round-trip or a
//! self-consistent check passes just as happily when the implementation is wrong.
//!
//! WGS84 constants used:
//! * a degree of **latitude** near the equator = 110574.4 m
//! * a degree of **longitude** at the equator  = 111319.5 m
//! * the spherical (Haversine) degree, R = 6371008.8 m (GRS80 mean radius,
//!   `HaversineMeasure::GRS80_MEAN_RADIUS`) = 111195.08 m
//!
//! The area/length split matters and is asserted: `ST_AREA` is **ellipsoidal**
//! (Karney's geodesic area), while `ST_LENGTH` / `ST_PERIMETER` are **spherical**
//! (Haversine). So a 1-degree meridian is 110574 m as an *area* factor but
//! 111195 m as a *length*. That is a real, deliberate difference in this
//! implementation, not a rounding artefact, and the tolerances here are tight
//! enough that swapping either one would fail.
//!
//! # Divergence from PostGIS, asserted deliberately
//!
//! PostGIS's `geometry` type returns **degrees** and **square degrees** on
//! EPSG:4326. RaisinDB has one geometry type and picks semantics from the SRID,
//! so these are **metres** and **square metres**, matching PostGIS's `geography`.
//! The assertions below encode OUR documented behaviour.

use super::harness::Ctx;

/// Metres per degree of longitude at the equator (WGS84 ellipsoid).
const DEG_LON_EQUATOR_M: f64 = 111319.49;
/// Metres per degree of latitude near the equator (WGS84 ellipsoid).
const DEG_LAT_EQUATOR_M: f64 = 110574.39;
/// Metres per degree on the GRS80 mean sphere — the Haversine degree.
const DEG_SPHERE_M: f64 = 111195.08;

pub async fn run(ctx: &mut Ctx) {
    println!("\n=== area ===");
    area(ctx).await;
    println!("\n=== length & perimeter ===");
    length(ctx).await;
    println!("\n=== distance ===");
    distance(ctx).await;
    println!("\n=== azimuth ===");
    azimuth(ctx).await;
}

mod area;
mod distance;

use area::area;
use distance::{azimuth, distance, length};
