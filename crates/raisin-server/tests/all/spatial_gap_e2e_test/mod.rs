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

//! The two spatial gaps, proven closed against a real server in one run.
//!
//! ```bash
//! cargo test -p raisin-server --test all spatial_gap_e2e_test -- --ignored --nocapture
//! ```
//!
//! # Gap 1 — write-time SRID normalisation
//!
//! Geohash cells are lon/lat **degrees**. Zurich in EPSG:3857 is
//! `(950_668, 6_002_678)`, which fails the encoder's `-180..=180 / -90..=90`
//! domain check, so the write produced *no cell at all*: the row was stored, the
//! index reported healthy, and it was invisible to every `ST_DWITHIN` forever
//! with nothing logged. Phases 1–3 write projected geometry over SQL DML and over
//! the REST node API, find it with a WGS84 query, prove a mixed-CRS workspace
//! answers correctly for both frames, and prove an SRID the built-in projection
//! tier cannot normalise is REJECTED rather than stored-and-unfindable.
//!
//! # Gap 3 — precision policy resolution
//!
//! The write path used to resolve its precision set from the LOCAL STATE RECORD,
//! so an operator who changed the policy changed nothing: writes kept emitting the
//! old precisions and nothing ever reconciled. Phases 4–6 change the policy over
//! `PUT …/spatial/config` and over SQL `ALTER`, and assert that (a) subsequent
//! writes emit the newly configured precisions, (b) a rebuild is really queued —
//! checked in the job queue, not by trusting the 2xx, (c) a configuration change
//! *alone* moves nothing, and (d) query results are byte-identical before, during
//! and after the migration, including a rebuild between two DISJOINT precision
//! sets, where the index is deliberately declared unusable and the planner must
//! fall back to a scan.
//!
//! # Why the whole thing is one test
//!
//! The phases share expensive state — a provisioned repo with a populated index —
//! and, more importantly, gap 3's assertions are only meaningful on a workspace
//! that gap 1's assertions already established is correctly indexed. Splitting
//! them would either duplicate the fixture (a second server, ~30 s) or leave the
//! policy phases asserting against an index nobody proved was right.

mod fixture;
mod observe;
mod policy_phases;
mod queries;
mod rebuild_window;
mod srid_phases;
mod transport;

use crate::helpers::multi_node::{ServerConfig, ServerHandle};

/// Not shared with any other test file in this binary.
const PORT: u16 = 8241;

/// Zurich, and the same physical place in EPSG:3857.
///
/// The easting is the point of the whole exercise: read as a longitude it is not
/// merely wrong, it is outside the domain, so the pre-fix write silently indexed
/// nothing at all.
pub const ZURICH: (f64, f64) = (8.54, 47.37);
pub const ZURICH_3857: (f64, f64) = (950_668.451_242_222_1, 6_002_677.996_697_136);

/// Bern, ~95 km from Zurich — far enough that a 5 km radius isolates one pair.
pub const BERN: (f64, f64) = (7.439_6, 46.949);
pub const BERN_3857: (f64, f64) = (828_172.483_590_355_5, 5_933_753.542_923_848);

/// Swiss LV95. A real CRS that `proj4rs-backend` / `proj-backend` both know, and
/// that index normalisation refuses on **every** build — widening it by Cargo
/// feature would let two nodes of one cluster hold different index bytes for the
/// same replicated record.
pub const LV95: (f64, f64, u32) = (2_683_000.0, 1_247_000.0, 2056);

#[tokio::test]
#[ignore] // starts a real server; run with --ignored
async fn spatial_srid_and_policy_end_to_end() {
    println!("\n=== spatial gaps 1 + 3, end to end ===\n");

    let server = ServerHandle::start(ServerConfig::new(PORT))
        .await
        .expect("failed to start server");
    // Admin user creation is asynchronous at boot.
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let base = server.base_url.clone();
    let token = fixture::bootstrap_admin(&base).await;
    fixture::provision(&base, &token).await;
    println!("[OK] server up; repo/workspace/nodetype provisioned");

    // ---- Gap 1 -----------------------------------------------------------
    srid_phases::projected_geometry_is_indexed(&base, &token).await;
    let baseline = srid_phases::mixed_srid_workspace_answers_both_frames(&base, &token).await;
    srid_phases::unindexable_srid_fails_loudly(&base, &token).await;

    // ---- Gap 3 -----------------------------------------------------------
    policy_phases::padding_rows(&base, &token, &baseline).await;
    policy_phases::http_config_change_reaches_writes_and_queues_a_rebuild(&base, &token, &baseline)
        .await;
    policy_phases::sql_config_change_alone_moves_nothing(&base, &token, &baseline).await;
    rebuild_window::disjoint_rebuild_never_answers_partially(&base, &token, &baseline).await;

    println!("\n=== spatial gaps 1 + 3: PASS ===\n");
}
