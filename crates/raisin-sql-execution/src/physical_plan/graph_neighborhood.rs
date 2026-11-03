// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Depth-aware neighbourhood of a node: the nodes within N hops, as a SET.
//!
//! "Friends of friends", "everyone within three introductions" and "is X within
//! two hops of me" are questions about which nodes are reachable, not about the
//! routes that reach them. Answering them by enumerating paths (what a
//! variable-length `GRAPH_TABLE` pattern does) costs one result per ROUTE: it
//! grows with the product of the degrees along the way, hits the path cap on a
//! hub, and is a poor fit for a question that wants each node once.
//!
//! This module answers them directly. It is a level-synchronous breadth-first
//! search over the per-node relation indexes:
//!
//! * every node is visited ONCE (a visited set), so the work is bounded by the
//!   size of the neighbourhood, not by the number of paths through it;
//! * each visited node costs one index seek of O(its degree), never a scan of
//!   the repository's relations;
//! * the result is one row per node with its SHORTEST distance (`depth`) and
//!   `via_count`, the number of distinct nodes one level closer that reach it.
//!   For depth 2 that is the number of mutual connections.
//!
//! # Limits fail, they do not truncate
//!
//! The search refuses to continue past `max_nodes` visited nodes and returns an
//! error saying so. A neighbourhood cut short is indistinguishable from a
//! complete one once it reaches the client, which is the failure shape the path
//! cap in `pgq::matching::traversal` already refuses.
//!
//! # Visibility
//!
//! The policy is the one `GRAPH_TABLE` already has (see `pgq::rls`): ENDPOINTS
//! ONLY. A node is checked by the [`NodeGate`] when it is about to be REPORTED;
//! a node that is merely walked through is not checked and stays traversable.
//! A caller therefore gets the nodes they may read that are within reach, and
//! `via_count` counts every intermediate, readable or not, exactly as a
//! `GRAPH_TABLE` path through an invisible middle node still produces its row.
//! Making the search stricter than the pattern matcher would give two answers to
//! "who is two hops from me" depending on which surface asked.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use super::executor::ExecutionError;
use super::graph_access::{NodeKey, RelationAccess, Side};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Deepest neighbourhood a query may ask for. Beyond a handful of hops the
/// neighbourhood of a connected graph is the whole graph.
pub const MAX_DEPTH: u32 = 16;

/// Default visited-node budget.
pub const DEFAULT_MAX_NODES: usize = 200_000;

/// Largest budget a query may set for itself.
pub const MAX_MAX_NODES: usize = 5_000_000;

/// What to compute.
#[derive(Debug, Clone)]
pub struct NeighborhoodParams {
    /// Which relations to follow from each node.
    pub side: Side,
    /// Relation types to follow; empty = every type. Exact, case-sensitive.
    pub types: Vec<String>,
    /// Shallowest depth to REPORT. Nodes nearer than this are still traversed.
    pub min_depth: u32,
    /// Deepest hop to follow.
    pub max_depth: u32,
    /// Visited-node budget, the start nodes included.
    pub max_nodes: usize,
}

/// Decides whether a node may be REPORTED. It is asked only for nodes at or
/// beyond `min_depth`, and never for the start nodes.
pub trait NodeGate: Send + Sync {
    /// What the gate hands back for a reportable node (the loaded node, say).
    type Item: Send;

    /// `Some(item)` lets the node into the result; `None` leaves it out. Either
    /// way the search walks on through it.
    fn report(
        &self,
        workspace: &str,
        id: &str,
        depth: u32,
    ) -> impl Future<Output = Result<Option<Self::Item>>> + Send;
}

/// One reported node.
#[derive(Debug)]
pub struct Hit<I> {
    pub workspace: String,
    pub id: String,
    /// Shortest distance from the nearest start node.
    pub depth: u32,
    /// Distinct nodes at `depth - 1` with a followed relation to this node.
    pub via_count: u64,
    pub item: I,
}

fn budget_error(max_nodes: usize, visited: usize) -> ExecutionError {
    ExecutionError::Validation(format!(
        "neighbourhood search visited more than max_nodes = {max_nodes} nodes ({visited} \
         so far). Partial results are not returned because they are indistinguishable from \
         a complete answer. Lower depth, restrict types, or raise max_nodes (at most \
         {MAX_MAX_NODES})."
    ))
}

/// The nodes a followed relation leads to from `node`.
async fn step<A: RelationAccess>(
    access: &A,
    node: &NodeKey,
    side: Side,
    types: &[String],
) -> Result<Vec<NodeKey>> {
    let (ws, id) = node;
    let mut out = Vec::new();
    let wanted = |t: &str| types.is_empty() || types.iter().any(|w| w == t);
    if matches!(side, Side::Out | Side::Both) {
        for rel in access.outgoing(ws, id).await? {
            if wanted(&rel.relation_type) {
                out.push((rel.workspace, rel.target));
            }
        }
    }
    if matches!(side, Side::In | Side::Both) {
        for (src_ws, src_id, rel) in access.incoming(ws, id).await? {
            if wanted(&rel.relation_type) {
                out.push((src_ws, src_id));
            }
        }
    }
    Ok(out)
}

/// The neighbourhood of `starts`. The start nodes themselves are never part of
/// the result.
///
/// Rows come back ordered by depth, then by `(workspace, id)`, so the result is
/// deterministic whatever order the index returned relations in.
pub async fn neighborhood<A: RelationAccess, G: NodeGate>(
    access: &A,
    gate: &G,
    starts: &[NodeKey],
    params: &NeighborhoodParams,
) -> Result<Vec<Hit<G::Item>>> {
    let mut visited: HashSet<NodeKey> = HashSet::new();
    let mut frontier: Vec<NodeKey> = Vec::new();

    for start in starts {
        if visited.insert(start.clone()) {
            frontier.push(start.clone());
        }
    }
    if visited.len() > params.max_nodes {
        return Err(budget_error(params.max_nodes, visited.len()));
    }

    let mut hits: Vec<Hit<G::Item>> = Vec::new();

    for depth in 1..=params.max_depth {
        if frontier.is_empty() {
            break;
        }

        // node -> (distinct frontier nodes reaching it, index of the last one
        // counted). Frontier nodes are processed one at a time, so comparing
        // against the last index is enough to count each of them once even when
        // two relations (or both directions) join the same pair.
        let mut reached: HashMap<NodeKey, (u64, usize)> = HashMap::new();

        for (idx, node) in frontier.iter().enumerate() {
            for next in step(access, node, params.side, &params.types).await? {
                if visited.contains(&next) {
                    continue;
                }
                let entry = reached.entry(next).or_insert((0, usize::MAX));
                if entry.1 != idx {
                    entry.0 += 1;
                    entry.1 = idx;
                }
            }
            if visited.len() + reached.len() > params.max_nodes {
                return Err(budget_error(
                    params.max_nodes,
                    visited.len() + reached.len(),
                ));
            }
        }

        let mut level: Vec<(NodeKey, u64)> =
            reached.into_iter().map(|(k, (n, _))| (k, n)).collect();
        level.sort();

        let mut next_frontier = Vec::with_capacity(level.len());
        for (key, via_count) in level {
            visited.insert(key.clone());
            if depth >= params.min_depth {
                if let Some(item) = gate.report(&key.0, &key.1, depth).await? {
                    hits.push(Hit {
                        workspace: key.0.clone(),
                        id: key.1.clone(),
                        depth,
                        via_count,
                        item,
                    });
                }
            }
            next_frontier.push(key);
        }
        frontier = next_frontier;
    }

    Ok(hits)
}

#[cfg(test)]
mod tests;
