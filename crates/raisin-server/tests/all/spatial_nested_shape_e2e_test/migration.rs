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

//! Sections 6–8 — per-property policy on nested paths, and the rebuild that is
//! the migration story for data written before nested indexing existed.

use super::fixture::Ctx;
use super::{dwithin, LAT, LON_C0_PIN, LON_C1_PIN, LON_HERO_STAGE, PATHS, WS};

/// `INDEX_PRECISIONS_DEFAULT` — what a property indexes at with no admin action.
const DEFAULT_PRECISIONS: [usize; 8] = [2, 4, 6, 7, 8, 9, 10, 11];

/// Deliberately DISJOINT from the default set, so any entry at these precisions
/// can only have been produced after the policy change — there is no way to
/// mistake a leftover for a migration.
const NESTED_TRACKING_SET: [usize; 2] = [3, 5];

/// Also disjoint from the default, and different from the set above so the two
/// declarations cannot be confused for one another.
const ARRAY_SET: [usize; 2] = [1, 12];

/// A precision policy declared on a **nested** path takes effect for that path
/// and leaves its siblings alone.
///
/// Policy keys are opaque strings all the way down — the state-record key builder
/// and the planner's availability check both go through `policy_key_for_path` —
/// so a dotted path is a legal key with no mechanism change. What has to be
/// proven is that the whole chain (config record → rebuild → physical entries →
/// planner) agrees on it.
pub async fn precision_policy_applies_to_a_nested_path(ctx: &Ctx) {
    println!("\n--- 6. per-property precision policy on a NESTED path ---");

    let before = ctx.health("hero.stage.geo").await;
    assert_eq!(
        before.indexed,
        DEFAULT_PRECISIONS.to_vec(),
        "precondition: the nested path starts on the workspace default: {before}"
    );
    assert_eq!(
        before.distinct_nodes, 2,
        "precondition: both venues are indexed at the 3-level path: {before}"
    );

    let response = ctx.put_config("hero.stage.geo", &NESTED_TRACKING_SET).await;
    assert_eq!(
        response["property"].as_str(),
        Some("hero.stage.geo"),
        "the config record must be keyed by the dotted path verbatim: {response}"
    );
    assert!(
        response["rebuild_job_id"].as_str().is_some(),
        "a disjoint policy change on a nested path must queue a rebuild: {response}"
    );

    let after = ctx
        .await_health("hero.stage.geo", "the nested path rebuilt at (3, 5)", |h| {
            h.settled_at(&NESTED_TRACKING_SET)
        })
        .await;
    assert_eq!(after.distinct_nodes, 2, "{after}");
    assert_eq!(
        after.at(3) + after.at(5),
        after.live_entries,
        "every live entry must sit at one of the configured precisions: {after}"
    );
    assert_eq!(
        after.at(11),
        0,
        "the old default precisions must be gone: {after}"
    );
    println!(
        "[PASS] 'hero.stage.geo' is now indexed at {:?} — {} entries over {} nodes",
        after.indexed, after.live_entries, after.distinct_nodes
    );

    // The knob has to leave the answer alone.
    let query = dwithin("hero.stage.geo", LON_HERO_STAGE, LAT, 1_000.0);
    ctx.expect_names(
        "the nested path still answers correctly under the cheaper profile",
        &query,
        &["v1"],
    )
    .await;
    ctx.expect_index_backed("'hero.stage.geo' after the policy change", &query)
        .await;

    // A sibling path must be untouched: a policy is per property, and prefix
    // inheritance is deliberately NOT a thing (`hero` does not configure
    // `hero.stage.geo`, and `hero.stage.geo` does not configure `hero.pin`).
    let sibling = ctx.health("hero.pin").await;
    assert_eq!(
        sibling.indexed,
        DEFAULT_PRECISIONS.to_vec(),
        "a sibling nested path must keep the workspace default: {sibling}"
    );
    println!(
        "[PASS] the sibling 'hero.pin' is unaffected: {:?}",
        sibling.indexed
    );
}

/// One declaration keyed `content[].pin` must configure EVERY concrete element of
/// the array.
///
/// `content.0.pin`, `content.1.pin`, … are unboundedly many keys for one modelled
/// field, so the array index is collapsed to `[]` when — and only when — a policy
/// is resolved. If the admin surface and the planner's availability check did not
/// normalise identically, the planner would look up a record that does not exist
/// and report the field unindexed forever: correct results, permanently slow, and
/// no error anywhere.
pub async fn a_wildcard_policy_key_configures_every_array_element(ctx: &Ctx) {
    println!("\n--- 7. one `content[].pin` declaration configures every array element ---");

    let response = ctx.put_config("content[].pin", &ARRAY_SET).await;
    assert_eq!(
        response["property"].as_str(),
        Some("content[].pin"),
        "the wildcard key must be stored normalised: {response}"
    );

    for concrete in ["content.0.pin", "content.1.pin"] {
        let h = ctx
            .await_health(
                concrete,
                &format!("'{concrete}' rebuilt at {ARRAY_SET:?}"),
                |h| h.settled_at(&ARRAY_SET),
            )
            .await;
        assert_eq!(
            h.configured,
            ARRAY_SET.to_vec(),
            "'{concrete}' must resolve the `content[].pin` declaration: {h}"
        );
        assert_eq!(h.distinct_nodes, 2, "{h}");
    }
    println!("[PASS] both array elements resolved the single `content[].pin` declaration");

    // Still correct, and still index-backed, at the new precisions.
    for (path, lon) in [("content.0.pin", LON_C0_PIN), ("content.1.pin", LON_C1_PIN)] {
        let query = dwithin(path, lon, LAT, 1_000.0);
        ctx.expect_names(
            &format!("'{path}' still answers under the array policy"),
            &query,
            &["v1"],
        )
        .await;
        ctx.expect_index_backed(&format!("'{path}' under the array policy"), &query)
            .await;
    }
}

/// The migration story: a full `POST …/spatial/rebuild` re-derives every nested
/// entry from the property tree.
///
/// The rebuild is `tombstone-then-re-emit`, so it first removes whatever is
/// physically present for each node and property and then writes the entries back
/// from `walk_geometries`. Everything the index holds for a nested path
/// afterwards was therefore produced by the REBUILD JOB, not by the writer — which
/// is exactly the code path an operator runs over data written before nested
/// indexing existed. A rebuild that still walked only the top level would leave
/// every nested path tombstoned and empty, and every assertion below would fail.
pub async fn a_full_rebuild_re_derives_every_nested_entry(ctx: &Ctx) {
    println!("\n--- 8. a full rebuild re-derives every nested entry (the migration path) ---");

    let job_id = ctx.rebuild(None).await;
    println!("  queued workspace-wide rebuild job {job_id}");

    for (path, lon) in PATHS {
        let h = ctx
            .await_health(path, &format!("'{path}' rebuilt"), |h| {
                h.phase != "building"
                    && !h.needs_rebuild
                    && h.distinct_nodes == 2
                    && h.live_entries == h.distinct_nodes * h.indexed.len() as u64
            })
            .await;
        assert!(
            h.live_entries > 0,
            "'{path}' has no entries after a tombstone-then-re-emit rebuild — the \
             rebuild job did not walk into the nested property tree, so this path \
             is now indexed as EMPTY and every query on it silently returns \
             nothing: {h}"
        );

        let query = dwithin(path, lon, LAT, 1_000.0);
        ctx.expect_names(
            &format!("'{path}' is findable after the rebuild"),
            &query,
            &["v1"],
        )
        .await;
        ctx.expect_index_backed(&format!("'{path}' after the rebuild"), &query)
            .await;
    }

    // The per-property policies set above must survive the rebuild rather than
    // being flattened back to the workspace default.
    let nested = ctx.health("hero.stage.geo").await;
    assert_eq!(
        nested.indexed,
        NESTED_TRACKING_SET.to_vec(),
        "a rebuild builds at the CONFIGURED policy, including for nested paths: {nested}"
    );
    let array = ctx.health("content.0.pin").await;
    assert_eq!(
        array.indexed,
        ARRAY_SET.to_vec(),
        "a rebuild must honour the `content[].pin` declaration: {array}"
    );
    println!(
        "[PASS] every nested path survived a full rebuild at its own policy \
         (hero.stage.geo={:?}, content.0.pin={:?})",
        nested.indexed, array.indexed
    );

    // A last cross-check that the rebuild did not smear paths together.
    let smeared = ctx
        .names(&dwithin("hero.stage.geo", LON_C0_PIN, LAT, 1_000.0))
        .await;
    assert!(
        smeared.is_empty(),
        "after the rebuild '{}' must still be its own namespace, got {smeared:?}",
        "hero.stage.geo"
    );
    println!("[PASS] paths remain separate namespaces after the rebuild (workspace '{WS}')");
}
