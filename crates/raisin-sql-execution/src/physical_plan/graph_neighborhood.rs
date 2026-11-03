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
//!   `via_count`, the number of distinct readable nodes one level closer that
//!   reach it. For depth 2 that is the number of mutual connections.
//!
//! # Limits fail, they do not truncate
//!
//! Three limits stop the search, and each one FAILS the query rather than
//! cutting the result short (a neighbourhood cut short is indistinguishable
//! from a complete one once it reaches the client, which is the failure shape
//! the path cap in `pgq::matching::traversal` already refuses):
//!
//! * `max_nodes` visited nodes, the start nodes included;
//! * a ceiling on relations READ from the per-node indexes (every relation a
//!   seek returns counts, followed or not), carried by a [`GraphBudget`];
//! * the budget's wall-clock deadline, checked after every seek and every
//!   visibility check.
//!
//! The error says nothing about how far the search got: no counts, no node
//! ids. Those numbers include nodes the caller may not read, so printing them
//! would let a caller measure what row-level security hides. They are logged
//! at debug level on the server instead.
//!
//! # Visibility
//!
//! * A START node the caller may not read is dropped before anything is read
//!   from it. If no start is left the result is empty, exactly as for a start
//!   that does not exist.
//! * A node is REPORTED only if the [`NodeGate`] lets it through.
//! * A node the caller may not read is still WALKED THROUGH. This is the
//!   endpoints-only policy `GRAPH_TABLE` has (see `pgq::rls`): a path through
//!   an invisible middle node still produces its row there, and the two
//!   surfaces must agree on who is two hops away. The consequence, the same
//!   one `GRAPH_TABLE` has, is that a reported node's `depth` can be shorter
//!   than any path made of readable nodes.
//! * `via_count` counts only previous-level nodes the caller may READ. Counting
//!   the hidden ones too would turn every row into a counter of hidden nodes
//!   next to it. A reported node reached only through hidden nodes therefore
//!   has `via_count = 0`.
//!
//! # Streaming
//!
//! [`NeighborhoodSearch`] hands the result out in chunks of at most
//! [`CHUNK`] nodes, level by level, so the executor turns each chunk into rows
//! before the next one is loaded. Only node KEYS are held for a whole level.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use super::executor::ExecutionError;
use super::graph_access::{NodeKey, RelationAccess, Side};
use super::graph_budget::{Exhausted, GraphBudget};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Deepest neighbourhood a query may ask for. Beyond a handful of hops the
/// neighbourhood of a connected graph is the whole graph.
pub const MAX_DEPTH: u32 = 16;

/// Default visited-node budget, and the most an ordinary caller gets.
pub const DEFAULT_MAX_NODES: usize = 200_000;

/// Largest budget a query may name. Only a system or system-admin caller is
/// given more than [`DEFAULT_MAX_NODES`]; see `neighborhood_scan`.
pub const MAX_MAX_NODES: usize = 1_000_000;

/// How many nodes are checked and handed out at a time.
pub const CHUNK: usize = 256;

/// The error for a search that ran into its node or relation limit.
pub const TOO_LARGE: &str =
    "neighbourhood too large: narrow the query with depth, types or max_nodes";

/// The error for a search that ran out of time.
pub const TOO_SLOW: &str =
    "neighbourhood search ran out of time: narrow the query with depth, types or max_nodes";

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
    /// Deadline and relation ceiling.
    pub budget: GraphBudget,
}

/// Decides whether the caller may read a node.
///
/// It is asked for every start node (`depth` 0), for every node at or beyond
/// `min_depth` (to report it) and for the level just before `min_depth` (so
/// `via_count` counts only readable nodes). Nodes nearer than that are walked
/// through without being asked about.
pub trait NodeGate: Send + Sync {
    /// What the gate hands back for a readable node (the loaded node, say).
    type Item: Send;

    /// `Some(item)` when the caller may read the node, `None` when not or when
    /// the node does not exist. Either way the search walks on through it.
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
    /// Distinct READABLE nodes at `depth - 1` with a followed relation to this
    /// node.
    pub via_count: u64,
    pub item: I,
}

/// Turn a stop into the error the caller sees. The numbers go to the log only.
fn refuse(why: Exhausted, visited: usize, budget: &GraphBudget) -> ExecutionError {
    tracing::debug!(
        reason = ?why,
        visited,
        relations_read = budget.relations_read(),
        relation_ceiling = budget.max_relations(),
        "neighbourhood search stopped at its budget"
    );
    ExecutionError::Validation(
        match why {
            Exhausted::Time => TOO_SLOW,
            Exhausted::Nodes | Exhausted::Relations => TOO_LARGE,
        }
        .to_string(),
    )
}

/// The nodes a followed relation leads to from `node`. Every relation the
/// index returns is charged, whether its type is followed or not.
async fn step<A: RelationAccess>(
    access: &A,
    node: &NodeKey,
    side: Side,
    types: &[String],
    budget: &mut GraphBudget,
) -> std::result::Result<Vec<NodeKey>, StepError> {
    let (ws, id) = node;
    let mut out = Vec::new();
    let wanted = |t: &str| types.is_empty() || types.iter().any(|w| w == t);
    if matches!(side, Side::Out | Side::Both) {
        let rels = access.outgoing(ws, id).await?;
        budget.charge(rels.len())?;
        for rel in rels {
            if wanted(&rel.relation_type) {
                out.push((rel.workspace, rel.target));
            }
        }
    }
    if matches!(side, Side::In | Side::Both) {
        let rels = access.incoming(ws, id).await?;
        budget.charge(rels.len())?;
        for (src_ws, src_id, rel) in rels {
            if wanted(&rel.relation_type) {
                out.push((src_ws, src_id));
            }
        }
    }
    Ok(out)
}

enum StepError {
    Failed(ExecutionError),
    Stopped(Exhausted),
}

impl From<ExecutionError> for StepError {
    fn from(e: ExecutionError) -> Self {
        Self::Failed(e)
    }
}

impl From<Exhausted> for StepError {
    fn from(e: Exhausted) -> Self {
        Self::Stopped(e)
    }
}

/// A breadth-first neighbourhood search that hands its result out in chunks.
///
/// Rows come out ordered by depth, then by `(workspace, id)`, so the result is
/// deterministic whatever order the index returned relations in. The start
/// nodes themselves are never part of the result.
pub struct NeighborhoodSearch<'a, A: RelationAccess, G: NodeGate> {
    access: &'a A,
    gate: &'a G,
    params: &'a NeighborhoodParams,
    budget: GraphBudget,
    visited: HashSet<NodeKey>,
    /// The level being handed out, `(node, via_count)` sorted by node.
    level: Vec<(NodeKey, u64)>,
    /// How much of `level` has been handed out.
    cursor: usize,
    /// Depth of `level`.
    depth: u32,
    /// Readable nodes of `level`, tracked where the next level needs them.
    readable: HashSet<NodeKey>,
    done: bool,
}

impl<'a, A: RelationAccess, G: NodeGate> NeighborhoodSearch<'a, A, G> {
    /// Check the start nodes and get ready to walk from the readable ones.
    pub async fn start(
        access: &'a A,
        gate: &'a G,
        starts: &[NodeKey],
        params: &'a NeighborhoodParams,
    ) -> Result<Self> {
        let budget = params.budget.clone();
        let mut level = Vec::new();
        let mut seen = HashSet::new();
        for start in starts {
            if !seen.insert(start.clone()) {
                continue;
            }
            budget
                .check_time()
                .map_err(|why| refuse(why, seen.len(), &budget))?;
            if gate.report(&start.0, &start.1, 0).await?.is_some() {
                level.push((start.clone(), 0));
            }
        }
        level.sort();
        let visited: HashSet<NodeKey> = level.iter().map(|(k, _)| k.clone()).collect();
        if visited.len() > params.max_nodes {
            return Err(refuse(Exhausted::Nodes, visited.len(), &budget));
        }
        Ok(Self {
            access,
            gate,
            params,
            budget,
            readable: visited.clone(),
            visited,
            cursor: level.len(),
            level,
            depth: 0,
            done: false,
        })
    }

    /// The next chunk of reported nodes, or `None` when the search is over.
    /// A returned chunk is never empty.
    pub async fn next_chunk(&mut self) -> Result<Option<Vec<Hit<G::Item>>>> {
        loop {
            if self.cursor < self.level.len() {
                let hits = self.check_chunk().await?;
                if !hits.is_empty() {
                    return Ok(Some(hits));
                }
                continue;
            }
            if self.done || !self.expand().await? {
                self.done = true;
                return Ok(None);
            }
        }
    }

    /// Ask the gate about the next [`CHUNK`] nodes of the current level.
    async fn check_chunk(&mut self) -> Result<Vec<Hit<G::Item>>> {
        let p = self.params;
        let depth = self.depth;
        let report = depth >= p.min_depth;
        // The next level is reported: its via_count needs to know which nodes
        // of this one are readable.
        let feeds_report = depth < p.max_depth && depth + 1 >= p.min_depth;
        if !report && !feeds_report {
            // Walked through, never asked about.
            self.cursor = self.level.len();
            return Ok(Vec::new());
        }
        let end = (self.cursor + CHUNK).min(self.level.len());
        let mut hits = Vec::new();
        for i in self.cursor..end {
            self.budget
                .check_time()
                .map_err(|why| refuse(why, self.visited.len(), &self.budget))?;
            let (key, via_count) = &self.level[i];
            let Some(item) = self.gate.report(&key.0, &key.1, depth).await? else {
                continue;
            };
            if feeds_report {
                self.readable.insert(key.clone());
            }
            if report {
                hits.push(Hit {
                    workspace: key.0.clone(),
                    id: key.1.clone(),
                    depth,
                    via_count: *via_count,
                    item,
                });
            }
        }
        self.cursor = end;
        Ok(hits)
    }

    /// Replace the handed-out level by the next one. `false` when there is none.
    async fn expand(&mut self) -> Result<bool> {
        let p = self.params;
        if self.depth >= p.max_depth || self.level.is_empty() {
            return Ok(false);
        }
        let frontier: Vec<NodeKey> = std::mem::take(&mut self.level)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let readable = std::mem::take(&mut self.readable);

        // node -> (distinct readable frontier nodes reaching it, index of the
        // last one counted). Frontier nodes are processed one at a time, so
        // comparing against the last index is enough to count each of them once
        // even when two relations (or both directions) join the same pair.
        let mut reached: HashMap<NodeKey, (u64, usize)> = HashMap::new();
        for (idx, node) in frontier.iter().enumerate() {
            let next = match step(self.access, node, p.side, &p.types, &mut self.budget).await {
                Ok(next) => next,
                Err(StepError::Failed(e)) => return Err(e),
                Err(StepError::Stopped(why)) => {
                    return Err(refuse(
                        why,
                        self.visited.len() + reached.len(),
                        &self.budget,
                    ))
                }
            };
            let counts = readable.contains(node);
            for next in next {
                if self.visited.contains(&next) {
                    continue;
                }
                let entry = reached.entry(next).or_insert((0, usize::MAX));
                if counts && entry.1 != idx {
                    entry.0 += 1;
                    entry.1 = idx;
                }
            }
            if self.visited.len() + reached.len() > p.max_nodes {
                return Err(refuse(
                    Exhausted::Nodes,
                    self.visited.len() + reached.len(),
                    &self.budget,
                ));
            }
        }

        let mut level: Vec<(NodeKey, u64)> =
            reached.into_iter().map(|(k, (n, _))| (k, n)).collect();
        level.sort();
        self.visited.extend(level.iter().map(|(k, _)| k.clone()));
        self.level = level;
        self.cursor = 0;
        self.depth += 1;
        Ok(!self.level.is_empty())
    }
}

/// The whole neighbourhood of `starts` at once. The executor streams with
/// [`NeighborhoodSearch`]; this is for callers that want a `Vec`.
pub async fn neighborhood<A: RelationAccess, G: NodeGate>(
    access: &A,
    gate: &G,
    starts: &[NodeKey],
    params: &NeighborhoodParams,
) -> Result<Vec<Hit<G::Item>>> {
    let mut search = NeighborhoodSearch::start(access, gate, starts, params).await?;
    let mut hits = Vec::new();
    while let Some(chunk) = search.next_chunk().await? {
        hits.extend(chunk);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests;
