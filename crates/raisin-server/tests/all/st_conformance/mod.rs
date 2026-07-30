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

//! End-to-end conformance suite for the whole `ST_*` library.
//!
//! ```bash
//! cargo test -p raisin-server --test all st_conformance -- --ignored --nocapture
//! ```
//!
//! # What "conformance" means here
//!
//! The standard is *"what is in there must work"*: if a function is registered
//! and documented, it must return **correct results against real stored data**.
//! Three things follow from that, and they shape the whole suite.
//!
//! **1. The function list is discovered, not written down.** `coverage.rs`
//! scans the analyzer's own registration source with `include_str!`, so adding a
//! function to `register_geospatial` without testing it turns into a printed
//! `*GAP*` line and a failed run. A hardcoded list is how a conformance suite
//! quietly stops being one — the brief that commissioned this file said "49
//! ST_* functions" and the real count was already 62.
//!
//! **2. Expected values are derived independently.** Areas come from the WGS84
//! degree lengths, `ST_TRANSFORM` from the closed form of Pseudo-Mercator,
//! DE-9IM matrices from the published JTS/PostGIS strings. A round trip proves
//! nothing: an inverted sign, a swapped axis pair and a wrong ellipsoid all
//! cancel when you go forward and back.
//!
//! **3. Failures accumulate instead of aborting.** Every assertion pushes into
//! `Ctx::failures` rather than panicking, and the run fails at the end with the
//! whole list. A suite that dies on failure #1 hides #2..#N, which is exactly
//! the information needed to judge release readiness.
//!
//! # Deliberate divergences from PostGIS, asserted as OUR behaviour
//!
//! PostGIS has two types (`geometry`, planar and unit-less; `geography`,
//! geodesic and metric). RaisinDB has one and selects semantics from the SRID,
//! so on EPSG:4326 it matches PostGIS's **`geography`**:
//!
//! * `ST_AREA` is square **metres** (PostGIS `geometry`: square degrees)
//! * `ST_LENGTH` / `ST_PERIMETER` are **metres** (PostGIS `geometry`: degrees)
//! * `ST_BUFFER(g, d)` and `ST_SIMPLIFY(g, t)` take **metres**
//! * topological predicates are **planar**, matching PostGIS `geometry` —
//!   straight edges in lon/lat, not great circles
//!
//! Each assertion that encodes a divergence says so at the call site.
//!
//! # A note on reading stored geometry in SQL
//!
//! `CAST(properties->>'g' AS GEOMETRY)` is the form that works.
//! `properties->>'g'` alone is `TEXT?` and no `ST_*` signature accepts it, and
//! `ST_GEOMFROMGEOJSON(properties->>'g'::String)` yields NULL when the nodetype
//! declares the property as `Geometry`. That asymmetry is worth its own look —
//! see the follow-ups in the report.

mod constructors;
mod coverage;
mod crs;
mod fixtures;
mod harness;
mod measures;
mod predicates;
mod processing;
mod stored;

use harness::Ctx;

/// The whole library, against one live server.
///
/// One test rather than several: booting a server, provisioning a repo and
/// inserting the corpus costs seconds, and a shared fixture set is what lets the
/// predicate matrix sweep every type pair cheaply.
#[tokio::test]
#[ignore = "starts a real server; run with --ignored"]
async fn st_star_conformance() {
    println!("\n############ ST_* CONFORMANCE SUITE ############");

    // Port chosen well clear of the 8081-8212 band the other server tests use;
    // two servers on one port silently interleave into the same database and the
    // run fails with connection errors rather than assertion failures.
    let mut ctx = Ctx::start(8317).await;

    constructors::run(&mut ctx).await;
    measures::run(&mut ctx).await;
    processing::run(&mut ctx).await;
    predicates::run(&mut ctx).await;
    crs::run(&mut ctx).await;
    stored::run(&mut ctx).await;

    // The coverage ledger is printed unconditionally, so a gap is visible even
    // on a passing run.
    let gaps = ctx.cov.report();

    if !ctx.failures.is_empty() {
        println!(
            "\n############ FAILURES ({}) ############",
            ctx.failures.len()
        );
        for (i, f) in ctx.failures.iter().enumerate() {
            println!("{:>3}. {f}", i + 1);
        }
    }

    // Reported, never swallowed — but deliberately not part of the verdict; see
    // the field docs on `Ctx::product_gaps`.
    if !ctx.product_gaps.is_empty() {
        println!(
            "\n######## PRODUCT GAPS OUTSIDE THE ST_* LIBRARY ({}) ########",
            ctx.product_gaps.len()
        );
        for (i, gap) in ctx.product_gaps.iter().enumerate() {
            println!("{:>3}. {gap}", i + 1);
        }
        println!("(these do not fail this suite — they are not ST_* correctness defects)");
    }

    let mut verdict = Vec::new();
    if !gaps.is_empty() {
        verdict.push(format!(
            "{} registered ST_* function(s) were never exercised: {:?}",
            gaps.len(),
            gaps
        ));
    }
    if !ctx.failures.is_empty() {
        verdict.push(format!("{} assertion(s) failed", ctx.failures.len()));
    }

    assert!(
        verdict.is_empty(),
        "ST_* conformance FAILED: {}",
        verdict.join("; ")
    );

    println!("\n############ ST_* CONFORMANCE PASSED ############");
}
