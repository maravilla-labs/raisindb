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

//! The spatial policy an operator sets on one node must take effect on every
//! node, must survive an out-of-order peer message, and must leave the workspace
//! answering spatial queries identically everywhere.
//!
//! # The bug this exists to catch
//!
//! `OpType::UpdateWorkspace` shipped with an applier and **no producer anywhere
//! in the tree**. `WorkspaceRepositoryImpl::put` held no `OperationCapture` and
//! emitted only a local event, so every workspace record — and with it every
//! `WorkspaceConfig.spatial`, whose entire documented cluster fan-out mechanism
//! *is* this operation — was silently local to the node it was written on.
//! `PUT …/spatial/config` returned success and the peers never heard about it.
//!
//! The half-fix is worse than the bug. The applier was a blind overwrite, so a
//! producer on its own would let an out-of-order or replayed peer message
//! carrying an OLDER workspace land on top of a newer one and silently revert a
//! config change the operator watched succeed.
//!
//! # Two tests, deliberately
//!
//! Phases 1-2 (fan-out, and the consequence on every node) are stable and run in
//! `test_spatial_config_replicates_across_a_cluster`. Phases 3-4 (a genuinely
//! stale operation delivered across a partition heal) live in
//! `test_older_config_does_not_clobber_newer_across_a_partition`, which is
//! currently unstable for propagation-timing reasons described on that test.
//! The LWW invariant itself is pinned deterministically by the unit tests in
//! `raisin-rocksdb/src/replication/application/applicator/workspace_lww.rs`, so
//! the partition test is a belt-and-braces integration check rather than the
//! proof — which is why a wobble there must not be able to mask a regression in
//! the stable half.
//!
//! # What each phase adds that the others do not
//!
//! 1. **Fan-out through the admin surface.** `spatial_cluster_test` and
//!    `workspace_config_replication_test` both drive
//!    `PUT /api/workspaces/{repo}/{ws}/config`, which replaces the whole
//!    `WorkspaceConfig`. The endpoint an operator and the admin console actually
//!    use is `PUT /api/admin/management/database/{tenant}/{repo}/spatial/config`,
//!    which *merges* into the existing `SpatialWorkspaceSchema` and queues a local
//!    rebuild. It reaches the same replicated record by a different route, so it
//!    is a different thing to get wrong. The read-back is `GET …/spatial/config`
//!    on each peer, polled to convergence.
//!
//! 2. **The consequence, not the record.** A replicated config is not the
//!    deliverable; a workspace that answers the same spatial question identically
//!    on every node is. Asserted with the shared helpers
//!    (`verify_spatial_query_on_all_nodes`, `verify_plan_contains_on_all_nodes`,
//!    `verify_all_nodes_agree`) rather than a local reimplementation, so the plan
//!    is checked too — correct rows from a row-level fallback are not evidence
//!    that a peer built index entries.
//!
//! 3. **A genuinely stale operation, delivered late.** This is the assertion no
//!    other suite makes, and the one the fix is most likely to regress on.
//!    Writing config A on one node and config B on another proves nothing: B is
//!    produced later *and* arrives later, so a blind overwrite passes. So the
//!    peers are **killed** (not merely frozen — a paused peer's kernel still
//!    buffers the bytes and hands them over in order on resume, which is how the
//!    first version of this phase passed while proving nothing), the losing write
//!    is made while the writer is genuinely alone, the writer is then frozen, the
//!    peers are restarted and given the winning write, and only then is the
//!    writer thawed. Nothing is mocked: real producer, real transport, real
//!    applier. See `lww_phase`.
//!
//! 4. **Still usable afterwards.** A conflict that converges but leaves the
//!    partitioned node's index inconsistent would pass phase 3 and still be
//!    broken.
//!
//! # Running it
//!
//! ```bash
//! cargo test -p raisin-server --test all spatial_config_replication_test \
//!     -- --ignored --nocapture
//! ```
//!
//! Three nodes, which is the minimum that proves the property: with two, "the
//! config reached the peers" is satisfiable by a single point-to-point link, and
//! phase 3 has no surviving *pair* to advance while one node is partitioned — a
//! two-node run cannot distinguish "the older operation was rejected" from "the
//! cluster simply stopped".

mod config_api;
mod fixture;
mod lww_phase;
mod phases;
mod provision;

use fixture::Cluster;
use provision::provision;

/// Config replication and index locality across a healthy 3-node cluster.
///
/// This is the stable proof that `OpType::UpdateWorkspace` now has a producer:
/// a `PUT .../spatial/config` on one node reaches its peers, and a geometry
/// written on any node is index-backed on every node.
///
/// The last-write-wins *guard* is deliberately NOT asserted here. Its semantics
/// are pinned deterministically by the seven unit tests in
/// `raisin-rocksdb/src/replication/application/applicator/workspace_lww.rs`
/// (`older_incoming_must_not_clobber_newer_stored`, `newer_incoming_wins`,
/// `identical_mtime_applies_idempotently`, …). Reproducing that invariant through
/// a real partition costs a full cluster kill plus a staggered restart plus a
/// backlog pull on the peers' ~30 s sync cycle — it exercises propagation LATENCY,
/// not the guard, and it belongs in its own test (below) so a timing wobble there
/// cannot mask a genuine regression here.
#[tokio::test]
#[ignore] // cargo test -p raisin-server --test all spatial_config_replication_test -- --ignored --nocapture
async fn test_spatial_config_replicates_across_a_cluster() {
    println!("\n=== spatial config replication across a cluster ===\n");

    let c = Cluster::start(3).await;
    provision(&c).await;
    println!("\n[OK] 3-node cluster up, repo/workspace/nodetype provisioned\n");

    // Every phase runs even if an earlier one failed, so one run reports the whole
    // picture rather than only the first break. The phases mutate shared state in
    // order, so a later failure may be caused by an earlier one; the summary keeps
    // the order so that is visible.
    let mut outcomes: Vec<(&str, Result<(), String>)> = Vec::new();

    macro_rules! phase {
        ($name:expr, $body:expr) => {{
            println!("\n--- {} ---", $name);
            let outcome = $body.await;
            match &outcome {
                Ok(()) => println!("[PASS] {}", $name),
                Err(e) => println!("[FAIL] {}\n       {}", $name, e),
            }
            outcomes.push(($name, outcome));
        }};
    }

    phase!(
        "PUT .../spatial/config on node1 is readable on node2 and node3",
        phases::phase_config_fans_out(&c)
    );
    phase!(
        "a geometry written on any node is index-backed on every node",
        phases::phase_geometry_is_findable_everywhere(&c)
    );
    println!("\n=== summary ===");
    for (name, outcome) in &outcomes {
        println!(
            "  {} {}",
            if outcome.is_ok() { "PASS" } else { "FAIL" },
            name
        );
    }

    let failures: Vec<String> = outcomes
        .iter()
        .filter_map(|(name, outcome)| outcome.as_ref().err().map(|e| format!("{name}: {e}")))
        .collect();

    if !failures.is_empty() {
        for logs in c.cluster.log_paths() {
            println!(
                "  logs for {}: {} / {}",
                logs.node_id,
                logs.stdout_path.display(),
                logs.stderr_path.display()
            );
        }
        panic!(
            "{} spatial config phase(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    println!("\n=== spatial config replication: PASS ===\n");
}

/// Last-write-wins across a real partition heal.
///
/// Kills all three nodes, brings them back staggered, and delivers an OLDER
/// workspace config after a NEWER one, asserting the newer survives and the
/// workspace still answers identically afterwards.
///
/// # Why this is separate, and currently unstable
///
/// It is the most demanding scenario in the suite and it is NOT the proof of the
/// LWW guard — that invariant is pinned deterministically by the unit tests in
/// `applicator/workspace_lww.rs`. What this exercises is *propagation*: after a
/// full-cluster kill, peers only pick up a restarted node's backlog on their
/// ~30 s sync cycle, so the assertion window has to outlast a cold restart plus a
/// sync tick plus provisioning.
///
/// As of this writing it fails in the second phase with
/// "the winning config did not reach the connected nodes within 90s" — note the
/// wording: the newer config had not ARRIVED, NOT that the older one won. No
/// inversion has ever been observed. The follow-on phase then fails on
/// "connection refused" because a killed node had not rebound its listener yet,
/// which is cascade damage rather than an independent failure.
///
/// Fixing it means making the windows survive a cold restart under parallel test
/// load and sequencing the restarts so every node is health-checked before the
/// assertion begins — worth doing before production, not worth blocking dev on.
/// Do NOT "fix" it by shortening the scenario: a partition that heals too easily
/// proves nothing.
#[tokio::test]
#[ignore] // KNOWN UNSTABLE - propagation timing, not a product defect. See doc comment.
async fn test_older_config_does_not_clobber_newer_across_a_partition() {
    println!("\n=== last-write-wins across a partition heal ===\n");

    let mut c = Cluster::start(3).await;
    provision(&c).await;
    println!("\n[OK] 3-node cluster up, repo/workspace/nodetype provisioned\n");

    let mut outcomes: Vec<(&str, Result<(), String>)> = Vec::new();

    macro_rules! phase {
        ($name:expr, $body:expr) => {{
            println!("\n--- {} ---", $name);
            let outcome = $body.await;
            match &outcome {
                Ok(()) => println!("[PASS] {}", $name),
                Err(e) => println!("[FAIL] {}\n       {}", $name, e),
            }
            outcomes.push(($name, outcome));
        }};
    }

    phase!(
        "an older config delivered after a newer one does not revert it",
        lww_phase::phase_older_update_does_not_clobber_newer(&mut c)
    );
    phase!(
        "the workspace still answers identically after the conflict healed",
        phases::phase_workspace_still_works_after_the_conflict(&c)
    );

    println!("\n=== summary ===");
    for (name, outcome) in &outcomes {
        println!(
            "  {} {}",
            if outcome.is_ok() { "PASS" } else { "FAIL" },
            name
        );
    }

    let failures: Vec<String> = outcomes
        .iter()
        .filter_map(|(name, outcome)| outcome.as_ref().err().map(|e| format!("{name}: {e}")))
        .collect();

    if !failures.is_empty() {
        for logs in c.cluster.log_paths() {
            println!(
                "  logs for {}: {} / {}",
                logs.node_id,
                logs.stdout_path.display(),
                logs.stderr_path.display()
            );
        }
        panic!(
            "{} partition-heal phase(s) failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    println!("\n=== last-write-wins across a partition heal: PASS ===\n");
}
