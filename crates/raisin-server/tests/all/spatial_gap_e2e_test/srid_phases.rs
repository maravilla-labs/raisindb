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

//! Gap 1: a geometry in a projected CRS is indexed in the WGS84 frame, and an
//! SRID this build cannot normalise is refused instead of silently stored.

use super::fixture::{all_site_names, insert_via_rest, insert_via_sql};
use super::observe::{await_health, health, sorted, verify};
use super::queries::{within, Baseline};
use super::transport::DEFAULT_PRECISIONS;
use super::{BERN, BERN_3857, LV95, ZURICH, ZURICH_3857};

/// The four sites: two places x two frames, over both write paths.
pub const SITES: [&str; 4] = [
    "bern-mercator",
    "bern-wgs84",
    "zurich-mercator",
    "zurich-wgs84",
];

/// Phase 1 — write four sites and prove all four reached the index.
///
/// The census is the assertion that would have caught the original bug on its
/// own: before write-time normalisation the two EPSG:3857 rows produced no cells
/// at all, so this reported two distinct nodes and called itself healthy.
pub async fn projected_geometry_is_indexed(base_url: &str, token: &str) {
    println!("\n--- 1. projected geometry reaches the index (SQL DML and REST) ---");

    insert_via_sql(base_url, token, "zurich-wgs84", ZURICH.0, ZURICH.1, None)
        .await
        .expect("SQL insert, unlabelled WGS84");
    insert_via_sql(
        base_url,
        token,
        "zurich-mercator",
        ZURICH_3857.0,
        ZURICH_3857.1,
        Some(3857),
    )
    .await
    .expect("SQL insert, EPSG:3857");
    insert_via_rest(base_url, token, "bern-wgs84", BERN.0, BERN.1, Some(4326))
        .await
        .expect("REST insert, explicit WGS84");
    insert_via_rest(
        base_url,
        token,
        "bern-mercator",
        BERN_3857.0,
        BERN_3857.1,
        Some(3857),
    )
    .await
    .expect("REST insert, EPSG:3857");

    let h = await_health(base_url, token, "all four sites indexed", |h| {
        h.distinct_nodes == 4 && h.live_entries == 4 * DEFAULT_PRECISIONS.len() as u64
    })
    .await;
    assert_eq!(
        h.distinct_nodes, 4,
        "the two EPSG:3857 rows must be indexed too; before write-time \
         normalisation their eastings failed the lon/lat domain check, produced no \
         cell at all, and this reported 2"
    );
    assert_eq!(
        h.indexed,
        sorted(DEFAULT_PRECISIONS.to_vec()),
        "a fresh workspace indexes at the default precision set with no admin action"
    );
    assert!(
        !h.needs_rebuild,
        "a freshly built index owes no rebuild: {h}"
    );

    let (status, detail) = verify(base_url, token).await;
    assert_eq!(status, "OK", "VERIFY on a freshly written index: {detail}");
    println!(
        "[PASS] 4 nodes x {} precisions; VERIFY OK",
        DEFAULT_PRECISIONS.len()
    );
}

/// Phase 2 — a mixed-SRID workspace answers correctly for BOTH frames.
///
/// Being *in* the index is not enough: the cells have to be where a WGS84 query
/// looks. Each place is stored twice, once per frame, so a 5 km radius must
/// return exactly the pair for that place — proving the projected row is neither
/// missing (the original bug) nor smeared somewhere else (a wrong normalisation
/// would put Zurich's easting/northing at some other lon/lat entirely, and the
/// Bern query would then be wrong too).
pub async fn mixed_srid_workspace_answers_both_frames(base_url: &str, token: &str) -> Baseline {
    println!("\n--- 2. mixed 4326 + 3857 workspace, both frames correct ---");

    let baseline = Baseline::capture(base_url, token).await;
    println!(
        "  5 km @ Zurich -> {:?}\n  5 km @ Bern   -> {:?}\n  400 km @ Bern -> {:?}",
        baseline.near_zurich, baseline.near_bern, baseline.wide
    );

    assert_eq!(
        baseline.near_zurich,
        vec!["zurich-mercator".to_string(), "zurich-wgs84".to_string()],
        "both Zurich rows are the same physical place, so a 5 km radius must \
         return the projected one as well as the WGS84 one"
    );
    assert_eq!(
        baseline.near_bern,
        vec!["bern-mercator".to_string(), "bern-wgs84".to_string()],
        "the Bern pair, and only the Bern pair, is within 5 km of Bern"
    );
    assert_eq!(
        baseline.wide,
        SITES.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "400 km around Bern reaches every site"
    );
    println!("[PASS] both CRS answer a WGS84 query, with no cross-contamination");
    baseline
}

/// Phase 3 — an SRID outside the built-in projection tier fails the WRITE.
///
/// This is the exact shape of the original bug, so the assertion is on the error
/// and on the absence of the row — not merely on "the query does not find it".
/// A stored-but-unindexable row is invisible to every spatial query forever, which
/// is strictly worse than a rejected write.
pub async fn unindexable_srid_fails_loudly(base_url: &str, token: &str) {
    println!("\n--- 3. an unindexable SRID is rejected, not silently unindexed ---");
    let (x, y, srid) = LV95;

    let via_sql = insert_via_sql(base_url, token, "lv95-sql", x, y, Some(srid))
        .await
        .err()
        .expect("EPSG:2056 must be REJECTED at write time, not silently unindexed");
    println!("  SQL:  {via_sql}");

    let via_rest = insert_via_rest(base_url, token, "lv95-rest", x, y, Some(srid))
        .await
        .err()
        .expect("EPSG:2056 must be rejected on the REST path too");
    println!("  REST: {via_rest}");

    for (path, message) in [("SQL", &via_sql), ("REST", &via_rest)] {
        assert!(
            message.contains("2056"),
            "the {path} error must name the offending SRID: {message}"
        );
        assert!(
            message.contains("proj4rs-backend") || message.contains("proj-backend"),
            "the {path} error must say what the feature flags do and do not change, \
             so nobody spends an afternoon enabling one: {message}"
        );
        assert!(
            message.starts_with("400"),
            "an unindexable SRID is a client error, not a 500: {message}"
        );
    }

    // Nothing was stored: neither row exists, and the index census is untouched.
    let stored = all_site_names(base_url, token).await;
    assert_eq!(
        stored,
        SITES.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "a rejected write must leave NO row behind — a stored row with an \
         unindexable SRID is precisely the invisible-forever state being fixed"
    );
    let h = health(base_url, token).await;
    assert_eq!(
        h.distinct_nodes, 4,
        "rejected writes must not touch the index: {h}"
    );

    // ...and the workspace still answers, i.e. the rejection did not poison it.
    assert_eq!(within(base_url, token, ZURICH, 5_000).await.len(), 2);
    println!("[PASS] EPSG:2056 rejected on both write paths; no row, no index entry");
}
