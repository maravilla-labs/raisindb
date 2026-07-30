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

//! The four phases. See `mod.rs` for what the suite as a whole is proving.

use super::config_api::{ascending, expect_everywhere, get_precisions, put_precisions};
use super::fixture::{dwithin, insert_place, Cluster, CENTER, CONVERGE, REPO};
use crate::cluster_test_utils::{
    verify_all_nodes_agree, verify_plan_contains_on_all_nodes, verify_spatial_query_on_all_nodes,
};

/// Deliberately disjoint from the server default `[11,10,9,8,7,6,4,2]` except at
/// 9, so a peer that merely fell back to defaults cannot pass by coincidence.
pub const FANNED_OUT: [usize; 4] = [3, 5, 9, 12];
/// The value written on the node that is about to be cut off. It must LOSE.
pub const STALE: [usize; 3] = [2, 4, 6];
/// Written later, on a connected node. It must WIN, everywhere and permanently.
pub const WINNER: [usize; 4] = [7, 8, 10, 11];

// -------------------------------------------------------------- phase 1: fan-out

/// A spatial policy written on node1 through `PUT …/spatial/config` must be
/// readable on node2 and node3 through `GET …/spatial/config`.
///
/// The peers are asserted, not the write's status code: a 204 on the writing node
/// proved nothing before the producer existed and proves nothing now. Only a read
/// on a *different* node distinguishes "replicated" from "written locally".
pub async fn phase_config_fans_out(c: &Cluster) -> Result<(), String> {
    let response = put_precisions(c, 0, &FANNED_OUT).await?;
    println!("  node1 PUT accepted: {response}");

    // The write must at least have taken effect where it was made, or the rest of
    // the phase is vacuous.
    let local = get_precisions(c, 0).await?;
    if local != ascending(&FANNED_OUT) {
        return Err(format!(
            "node1 did not persist the policy it was given; GET returned {local:?}"
        ));
    }
    println!("  node1 accepted and persisted the new precision set");

    expect_everywhere(c, &FANNED_OUT).await?;
    println!("  the new precision set reached node2 and node3");
    Ok(())
}

// ------------------------------------------------ phase 2: the query consequence

/// A geometry written on any node must be findable by an **index-backed** query
/// on every node, under the replicated policy.
///
/// The config replicating is not the deliverable; a workspace that answers the
/// same spatial question identically everywhere is. Both directions are written:
/// node1 originated the config, node3 only received it.
pub async fn phase_geometry_is_findable_everywhere(c: &Cluster) -> Result<(), String> {
    c.sql(0, &insert_place("origin1", CENTER.0, CENTER.1 + 0.000_30))
        .await?;
    c.sql(2, &insert_place("peer3", CENTER.0, CENTER.1 + 0.000_60))
        .await?;

    // Record convergence first. Without it a spatial miss below would be
    // ambiguous between "the index is missing" and "replication is slow".
    c.wait_for_record("origin1").await?;
    c.wait_for_record("peer3").await?;
    println!("  both records converged on all nodes");

    let near = dwithin(CENTER.0, CENTER.1, 100.0);
    verify_spatial_query_on_all_nodes(
        &c.client,
        &c.tokens,
        REPO,
        &near,
        &["origin1", "peer3"],
        CONVERGE,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;

    // The radius must discriminate; a query returning everything proves nothing
    // about an index.
    verify_spatial_query_on_all_nodes(
        &c.client,
        &c.tokens,
        REPO,
        &dwithin(CENTER.0, CENTER.1, 5.0),
        &[],
        CONVERGE,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;

    // Rows alone are not proof: a fail-closed planner that fell back to a row
    // filter would also return the right rows, just slowly. The plan is what says
    // the peers built their own entries under the replicated policy.
    verify_plan_contains_on_all_nodes(&c.client, &c.tokens, REPO, &near, "SpatialDistanceScan")
        .await
        .map_err(|e| format!("{e:#}"))?;
    println!("  every node's plan is a SpatialDistanceScan and every node agrees");
    Ok(())
}

// ------------------------------------- phase 4: usable after the conflict healed

/// After the conflict resolves, the workspace must still be usable and still
/// answer identically everywhere.
///
/// A config that converges but leaves the workspace broken on the node that lost
/// the conflict — or an index left half-written under the value that lost — would
/// pass phase 3 and still be a broken feature. Writes originate on the node that
/// was partitioned, which is the one most likely to be inconsistent.
pub async fn phase_workspace_still_works_after_the_conflict(c: &Cluster) -> Result<(), String> {
    c.sql(0, &insert_place("healed", CENTER.0, CENTER.1 + 0.000_45))
        .await?;
    c.wait_for_record("healed").await?;

    let near = dwithin(CENTER.0, CENTER.1, 100.0);
    verify_spatial_query_on_all_nodes(
        &c.client,
        &c.tokens,
        REPO,
        &near,
        &["healed", "origin1", "peer3"],
        CONVERGE,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;
    verify_plan_contains_on_all_nodes(&c.client, &c.tokens, REPO, &near, "SpatialDistanceScan")
        .await
        .map_err(|e| format!("{e:#}"))?;

    // A radius finer than the row spacing: `healed` sits ~17 m from `origin1` and
    // ~17 m from `peer3`, so a 10 m circle around it isolates it. This is where a
    // node whose index was rebuilt under the losing policy would diverge.
    let tight = dwithin(CENTER.0, CENTER.1 + 0.000_45, 10.0);
    verify_spatial_query_on_all_nodes(&c.client, &c.tokens, REPO, &tight, &["healed"], CONVERGE)
        .await
        .map_err(|e| format!("{e:#}"))?;

    // And the cross-node agreement assertion in its sharpest form: whatever the
    // answer to a wider query is, it must be the same answer on all three.
    let agreed = verify_all_nodes_agree(
        &c.client,
        &c.tokens,
        REPO,
        &dwithin(CENTER.0, CENTER.1, 1_000.0),
        CONVERGE,
    )
    .await
    .map_err(|e| format!("{e:#}"))?;
    println!("  every node agrees at 1 km after the conflict healed: {agreed:?}");

    // The surviving config must still be the winner after all this write traffic.
    expect_everywhere(c, &WINNER).await?;
    println!("  the winning policy is still in place after the workspace was written to");
    Ok(())
}
