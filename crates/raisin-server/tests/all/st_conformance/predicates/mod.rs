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

//! The ten DE-9IM topological predicates, `ST_RELATE`, and the algebraic
//! identities that tie them together.
//!
//! Predicates are **planar in the geometry's own coordinate space** — straight
//! edges in lon/lat, not great circles — matching PostGIS's `geometry` type. So
//! every expected value here is the plain planar answer and can be read off the
//! picture.
//!
//! # Why the type matrix, not one case per function
//!
//! The old implementation hand-wrote a match arm per geometry-type *pair*, so
//! each predicate supported a different, incomplete set. `ST_CROSSES` and
//! `ST_TOUCHES` hardcoded `false` for any Point argument; `ST_OVERLAPS` had a
//! catch-all `false` covering 46 of the 49 type pairs; pairs nobody wrote raised
//! "not supported for X and Y". A test with one example per function passes
//! throughout all of that. Sweeping the matrix is what makes it visible.

use super::harness::Ctx;

/// The ten predicates, by name.
pub(super) const PREDICATES: &[&str] = &[
    "ST_INTERSECTS",
    "ST_DISJOINT",
    "ST_CONTAINS",
    "ST_WITHIN",
    "ST_COVERS",
    "ST_COVEREDBY",
    "ST_TOUCHES",
    "ST_CROSSES",
    "ST_OVERLAPS",
    "ST_EQUALS",
];

/// Every geometry type, as a corpus label — both sides of the matrix.
pub(super) const ALL_TYPES: &[&str] = &["pt", "ls", "poly_hole", "mpt", "mls", "mpoly", "gc"];

pub async fn run(ctx: &mut Ctx) {
    println!("\n=== predicates: concrete expected values ===");
    concrete(ctx).await;
    println!("\n=== predicates: previously-broken cases (regressions) ===");
    regressions(ctx).await;
    println!("\n=== predicates: boundary cases, COVERS vs CONTAINS ===");
    boundary(ctx).await;
    println!("\n=== ST_RELATE ===");
    relate(ctx).await;
    println!("\n=== predicates: type matrix (all 49 pairs x 10 predicates) ===");
    type_matrix(ctx).await;
    println!("\n=== predicates: algebraic identities ===");
    identities(ctx).await;
}

/// The unit square, and shapes positioned against it.
pub(super) const SQUARE: &str =
    r#"{"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]}"#;

mod cases;
mod matrix;

use cases::{boundary, concrete, regressions, relate};
use matrix::{identities, type_matrix};
