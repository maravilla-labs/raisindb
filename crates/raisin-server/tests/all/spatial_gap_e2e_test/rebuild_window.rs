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

//! The rebuild window: what a query may see while the index migrates.

use super::observe::{health, put_config, verify};
use super::policy_phases::{DISJOINT_SET, FILLERS};
use super::queries::Baseline;

/// Phase 7 — a rebuild between two DISJOINT precision sets never answers partially.
///
/// Mid-rebuild the index holds a mixture of two policies, and with disjoint sets
/// no precision is complete for every row. The engine's rule is to declare the
/// index unusable and let the planner fall back to a scan with the predicate
/// retained — slow and correct, never fast and wrong. The proof is that the three
/// answers are identical on EVERY observation across the whole window.
pub async fn disjoint_rebuild_never_answers_partially(
    base_url: &str,
    token: &str,
    baseline: &Baseline,
) {
    println!("\n--- 7. disjoint rebuild: no partial answers, ever ---");

    // The rebuild of a few dozen rows takes ~10 ms, so a poll loop that starts
    // AFTER the config change usually misses the window entirely. The queries are
    // therefore already in flight when the policy changes: this hammer runs
    // continuously from just before the `PUT` until the rebuild has settled, and
    // every single answer it gets has to match the baseline.
    let hammer = {
        let base = base_url.to_string();
        let token = token.to_string();
        let expected = baseline.clone();
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
            let mut seen = 0usize;
            let mut wrong: Vec<String> = Vec::new();
            while std::time::Instant::now() < deadline {
                let now = Baseline::capture(&base, &token).await;
                seen += 1;
                if now.near_zurich != expected.near_zurich
                    || now.near_bern != expected.near_bern
                    || now.wide != expected.wide
                {
                    wrong.push(format!(
                        "zurich={:?} bern={:?} wide={:?}",
                        now.near_zurich, now.near_bern, now.wide
                    ));
                }
            }
            (seen, wrong)
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let response = put_config(base_url, token, &DISJOINT_SET).await;
    assert!(
        response["rebuild_job_id"].as_str().is_some(),
        "a disjoint policy change must queue a rebuild: {response}"
    );

    let mut saw_building = false;
    let mut observations = 0;
    let mut settled = None;
    let mut last = health(base_url, token).await;
    for _ in 0..80 {
        let h = health(base_url, token).await;
        if h.phase == "building" {
            saw_building = true;
        }
        // The invariant, asserted on every single observation of the window.
        let now = Baseline::capture(base_url, token).await;
        if now.near_zurich != baseline.near_zurich
            || now.near_bern != baseline.near_bern
            || now.wide != baseline.wide
        {
            println!("[DIAG] health at the divergence: {h}");
            println!(
                "[DIAG] 5 km @ Zurich EXPLAIN:\n{}",
                super::queries::explain(base_url, token, super::ZURICH, 5_000).await
            );
            panic!(
                "answers changed during the disjoint rebuild:\n  5 km @ Zurich {:?} != {:?}\n  \
                 5 km @ Bern {:?} != {:?}\n  400 km @ Bern {:?} != {:?}",
                now.near_zurich,
                baseline.near_zurich,
                now.near_bern,
                baseline.near_bern,
                now.wide,
                baseline.wide
            );
        }
        observations += 1;
        last = h.clone();
        if h.settled_at(&DISJOINT_SET) {
            settled = Some(h);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let h = settled
        .unwrap_or_else(|| panic!("the disjoint rebuild never settled; last health = {last}"));

    let want = (6 + FILLERS) as u64;
    assert_eq!(h.distinct_nodes, want, "{h}");
    assert_eq!(h.at(11), 0, "the old precisions must be gone: {h}");
    assert_eq!(
        h.at(10),
        want,
        "the new precisions must cover every row: {h}"
    );
    let (status, detail) = verify(base_url, token).await;
    assert_eq!(status, "OK", "VERIFY after the disjoint rebuild: {detail}");

    let (hammered, wrong) = hammer.await.expect("the query hammer panicked");
    assert!(
        wrong.is_empty(),
        "{} of {hammered} concurrent observations saw a different answer while the \
         index was migrating between two disjoint precision sets; first: {:?}",
        wrong.len(),
        wrong.first()
    );

    println!(
        "[PASS] {observations} polled + {hammered} concurrent observations across the \
         window, identical answers every time (phase=building seen: {saw_building})"
    );
    if !saw_building {
        println!(
            "[NOTE] the `Building` phase was never caught by the slower health poll — \
             the rebuild of this many rows takes ~10 ms. The invariant held on every \
             observation including the concurrent ones, and the mid-rebuild \
             availability rule itself is unit-tested in \
             raisin-rocksdb spatial_state::resolve."
        );
    }
}
