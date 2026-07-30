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

//! Phase 3: an older workspace update, delivered after a newer one.
//!
//! In its own file because it is the only phase that manipulates the cluster
//! topology, and because staging a genuinely stale delivery takes more
//! explanation than the assertions it supports.

use super::config_api::{ascending, expect_everywhere, get_precisions, put_precisions};
use super::fixture::{Cluster, LogMark, CONVERGE, REPO, WS};
use super::phases::{FANNED_OUT, STALE, WINNER};

use std::time::Duration;

/// How long the stale record gets to be delivered once node1 rejoins.
///
/// A returning node's backlog is pulled by its peers on their periodic sync, not
/// pushed on reconnect, so this must comfortably exceed that cycle (~30s).
const GUARD_DELIVERY: Duration = Duration::from_secs(150);

/// An **older** workspace update, delivered **after** a newer one, must not
/// revert it.
///
/// This is the case that makes a naive producer worse than no replication at all:
/// the operator watches `PUT …/spatial/config` succeed, the value propagates, and
/// then a reconnecting node's backlog silently rolls it back.
///
/// The delivery order is created, not hoped for, and every node is either fully
/// up or fully down while it is created. Two earlier designs failed for reasons
/// worth recording, because both look reasonable:
///
/// * **Pausing the peers with `SIGSTOP` proves nothing.** A stopped process still
///   owns its socket, so the kernel accepts and buffers the replication bytes on
///   its behalf and hands them over — in send order — the instant it is thawed.
///   The stale record was therefore delivered *before* the winning write, the
///   applier saw two in-order updates, and the guard was never reached. Only
///   killing a peer closes its socket and makes the sender's delivery genuinely
///   fail, leaving the operation in the sender's oplog.
/// * **Pausing the *writer* while its peers restart wedges the restart.** A frozen
///   node still accepts TCP connections it will never answer, so a peer booting
///   next to it burns a 30-second handshake timeout per retry — and observably
///   never reaches its own `listening on http://…`, binding only the replication
///   port. That is a startup-ordering weakness in the server (reported, not
///   chased); for this test it is enough that no node is ever left frozen.
///
/// So every transition here is a kill or a start. All three nodes go down, node1
/// alone comes up to make the losing write, node1 goes down again holding it,
/// the peers come up and take the winning write, and only then does node1 return
/// — reconnecting and draining a genuinely stale operation into a cluster that
/// has already moved on. That is also the most realistic shape of the bug.
///
/// Convergence is asserted in BOTH directions: node1 must adopt node2's newer
/// value (it lost), and node2/node3 must reject node1's older one (they won).
pub async fn phase_older_update_does_not_clobber_newer(c: &mut Cluster) -> Result<(), String> {
    // 1. Take the peers down for real, and leave them down. Freezing is not
    //    enough; see the doc comment.
    c.cluster
        .kill_node(1)
        .map_err(|e| format!("could not kill node2: {e:#}"))?;
    c.cluster
        .kill_node(2)
        .map_err(|e| format!("could not kill node3: {e:#}"))?;

    // 2. The losing write, made on a node whose peers cannot receive it.
    put_precisions(c, 0, &STALE)
        .await
        .map_err(|e| format!("the losing write was rejected: {e}"))?;

    // 3. Take node1 down too, trapping its undelivered operation in its oplog.
    //    Without this it would ship the moment the peers came back, before the
    //    winning write existed.
    c.cluster
        .kill_node(0)
        .map_err(|e| format!("could not kill node1: {e:#}"))?;
    println!("  node1 is down holding an undelivered {STALE:?}");

    // 4. Bring the peers back on their own data. They must still be on the phase-1
    //    value: if they had somehow received the losing write, the conflict this
    //    phase stages would not exist and everything below would be vacuous.
    c.restart(1).await?;
    c.restart(2).await?;
    for node in [1, 2] {
        let got = get_precisions(c, node).await?;
        if got != ascending(&FANNED_OUT) {
            return Err(format!(
                "node{} came back on {got:?}, not the pre-partition {FANNED_OUT:?} — the \
                 losing write was not actually withheld, so this phase cannot stage a \
                 stale delivery",
                node + 1
            ));
        }
    }
    println!("  node2 and node3 are back, still on the pre-partition {FANNED_OUT:?}");

    // A moment of real time, so "node2 wrote later" is true by wall clock and not
    // only by program order — the guard's comparator is the record's mtime.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 5. The winning write, into the surviving pair.
    put_precisions(c, 1, &WINNER)
        .await
        .map_err(|e| format!("the winning write was rejected: {e}"))?;

    // It must reach node3 before node1 returns, or the race is not the one under
    // test. node1 is down, so only the surviving pair can be polled.
    wait_for_pair(c, &[1, 2], &WINNER).await?;
    println!("  node2's newer {WINNER:?} is on both live nodes while node1 is still down");

    // 6. Bring node1 back, releasing its stale operation into a cluster that has
    //    moved on.
    //
    //    The mark is taken HERE, and after node2/node3's respawn truncated their
    //    logs: everything earlier is provisioning and phase 1/2 traffic, and the
    //    system workspaces genuinely do produce LWW rejections while a repository
    //    is created on three nodes at once. Only what happens from now on is
    //    evidence about the conflict this phase staged.
    let mark = c.log_mark();
    c.restart(0).await?;
    println!("  node1 is back — its older operation is now in flight");

    // 7. Every node must converge on the newer value — including node1, which
    //    produced the older one and has just rejoined.
    expect_everywhere(c, &WINNER).await?;
    println!("  the newer config is on every node, node1 included");

    // 8. Now wait for the stale operation to actually be DELIVERED, and require
    //    that the guard is what stops it.
    //
    //    Waiting is the point. A returning node does not shove its backlog at its
    //    peers; they pull it on their periodic sync, which runs on a ~30-second
    //    cycle. An earlier version asserted a fixed 15-second settle and reported
    //    "no node rejected anything" — not because the guard was broken but
    //    because the stale record had not been asked for yet. A fixed settle here
    //    tests the sync interval, not the guard.
    //
    //    The config is re-checked on every iteration, so this doubles as a much
    //    stronger stability assertion than a single settle: the winning value must
    //    hold across the entire delivery window, including the instant the stale
    //    record lands.
    expect_guard_rejection_while_stable(c, &mark).await
}

/// Poll until some node logs an LWW rejection for this suite's workspace, failing
/// if the winning config is ever seen to revert in the meantime.
async fn expect_guard_rejection_while_stable(c: &Cluster, mark: &LogMark) -> Result<(), String> {
    // The needle is the FULL `repo/workspace` scope, for two reasons. The system
    // workspaces (`functions`, `packages`, `raisin:system`, …) reject constantly
    // while a repository is created on three nodes at once, so a bare "a rejection
    // happened" match is satisfied no matter what this phase did — the first
    // version of this suite reported 19 rejections and not one was for the
    // workspace it had partitioned. And `ClusterProcess` log files used to be named
    // after the node id alone in a shared temp dir, so two cluster suites running
    // at once wrote to the SAME files; `spatial_cluster_test` also uses a workspace
    // called `places`, and a bare `/places)` would happily match its rejections.
    //
    // No trailing paren: the rejection logs the scope as `(tenant/repo/ws)` but the
    // apply logs it as `tenant/repo/ws from node …`, and both are matched here.
    let scope = format!("{REPO}/{WS}");
    let want = ascending(&WINNER);
    let deadline = tokio::time::Instant::now() + GUARD_DELIVERY;

    loop {
        for node in 0..c.nodes() {
            let got = get_precisions(c, node).await?;
            if got != want {
                return Err(format!(
                    "node{} reverted to {got:?} while the stale record was in flight — \
                     an older UpdateWorkspace clobbered a newer one",
                    node + 1
                ));
            }
        }

        let rejections = c.log_lines_since(mark, &["Ignoring older UpdateWorkspace", &scope]);
        if !rejections.is_empty() {
            println!(
                "  the LWW guard rejected the stale '{WS}' record {} time(s); first: {}",
                rejections.len(),
                rejections[0]
            );
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            let delivered = c.log_lines_since(mark, &["Applying workspace update", &scope]);
            return Err(format!(
                "the newer config held for {}s, but no node ever rejected an older '{WS}' \
                 record, so the LWW guard was never exercised — this run proves convergence \
                 and NOT the guard. Workspace updates delivered since node1 returned: {}",
                GUARD_DELIVERY.as_secs(),
                if delivered.is_empty() {
                    "none at all (node1's stale operation was never re-shipped)".to_string()
                } else {
                    format!("{delivered:#?}")
                }
            ));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Poll only `nodes` (the live ones) until each reports `want`.
///
/// A node that is deliberately down answers nothing, so `expect_everywhere` cannot
/// be used while the partition is open.
async fn wait_for_pair(c: &Cluster, nodes: &[usize], want: &[usize]) -> Result<(), String> {
    let want = ascending(want);
    let deadline = tokio::time::Instant::now() + CONVERGE;
    loop {
        let mut pending = Vec::new();
        for &node in nodes {
            match get_precisions(c, node).await {
                Ok(got) if got == want => {}
                Ok(got) => pending.push(format!("node{} has {got:?}", node + 1)),
                Err(e) => pending.push(format!("node{} unreadable: {e}", node + 1)),
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "the winning config did not reach the connected nodes within {}s \
                 (wanted {want:?}): {}",
                CONVERGE.as_secs(),
                pending.join("; ")
            ));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
