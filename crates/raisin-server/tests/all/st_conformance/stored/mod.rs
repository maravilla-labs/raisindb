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

//! The ST_* library applied to geometry **read out of stored nodes**.
//!
//! Everything else in this suite evaluates literals. That proves the maths but
//! not the plumbing: a geometry has to survive a SQL INSERT, the property-value
//! type inference, MessagePack storage, the scan, and the projection before an
//! `ST_*` call ever sees it. This module is the part that would catch a break in
//! any of those.

use super::harness::{Ctx, NODE_TYPE, WORKSPACE};

/// The stored geometry of one fixture, as an ST_* argument.
///
/// `CAST(properties->>'g' AS GEOMETRY)` is the working form. Note that
/// `ST_GEOMFROMGEOJSON(properties->>'g'::String)` yields NULL for a property the
/// nodetype declares as `Geometry` — see the note in `mod.rs`.
pub(super) const G: &str = "CAST(properties->>'g' AS GEOMETRY)";

pub(super) fn row_sql(label: &str, expr_sql: &str) -> String {
    format!(
        "SELECT {expr_sql} AS r FROM '{WORKSPACE}' \
         WHERE node_type = '{NODE_TYPE}' AND properties->>'label'::String = '{label}'"
    )
}

pub async fn run(ctx: &mut Ctx) {
    println!("\n=== stored geometry: round trip ===");
    round_trip(ctx).await;
    println!("\n=== stored geometry: functions over stored data ===");
    functions(ctx).await;
    println!("\n=== stored geometry: multiple ST_* in ONE projection ===");
    multi_projection(ctx).await;
    println!("\n=== stored geometry: predicates in WHERE ===");
    where_clause(ctx).await;
    println!("\n=== stored geometry: SQL UPDATE ===");
    update(ctx).await;
    println!("\n=== stored geometry: malformed input must fail loudly ===");
    malformed(ctx).await;
}

/// Every fixture reads back with the type it was written with.
mod gaps;
mod queries;
mod roundtrip;

use gaps::malformed;
use queries::{multi_projection, update, where_clause};
use roundtrip::{functions, round_trip};
