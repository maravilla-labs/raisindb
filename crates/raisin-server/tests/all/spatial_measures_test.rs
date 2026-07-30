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

//! Measurement, set operations, processing and validation end to end.
//!
//! **This module holds NO tests. The coverage described below was delivered in
//! [`crate::st_conformance`] instead — see `st_conformance/measures/`,
//! `st_conformance/processing/` (buffer, setops, validation) and
//! `st_conformance/stored/`.** Run it with:
//!
//! ```text
//! cargo test -p raisin-server --test all st_conformance -- --ignored --nocapture
//! ```
//!
//! This file is kept only for the trap list below, which records the specific
//! ways each of these functions can be plausibly wrong. Do not read a passing
//! `spatial_measures_test` run as evidence of anything: it selects zero tests.
//!
//! Every function against every geometry type its signature admits, against a
//! real server. The specific traps this must pin down:
//!
//! * **Units.** On a geographic CRS `ST_AREA` is square metres, `ST_LENGTH` and
//!   `ST_PERIMETER` are metres, and `ST_BUFFER`/`ST_SIMPLIFY` take metres. That
//!   diverges from PostGIS's `geometry` type (which returns degrees) and matches
//!   its `geography` type. Assert the magnitudes, not just success.
//! * **`ST_BUFFER` is the easiest thing here to get plausibly wrong.** `geo`'s
//!   `Buffer` is planar and works in the geometry's own units, so on EPSG:4326 a
//!   bare `.buffer(50)` means 50 DEGREES. Assert a buffered radius in metres.
//! * **`ST_DISTANCE` on non-point pairs.** `geo` implements geodesic `Distance`
//!   for Point-to-Point only, so the old centroid-to-centroid fallback is not
//!   fixable by swapping traits — it needs a projection to a metric CRS. Assert
//!   a polygon-to-polygon distance that a centroid approximation would get wrong.
//! * **`ST_ISVALID` on a self-intersecting bowtie** must be false. The old
//!   array-shape check passed it.
//! * **`ST_ISSIMPLE`** must not be a constant `true`.
//! * **`ST_AREA(ST_UNION(a, b))`** must work when the union yields a
//!   `MultiPolygon` — the named failure of the old implementation.
//! * **3-D:** `ST_Z`, `ST_ZMIN`/`ST_ZMAX`, `ST_NDIMS`, `ST_FORCE2D`/`ST_FORCE3D`,
//!   `ST_3DDISTANCE`, `ST_3DDWITHIN`, on data written with a third ordinate.
