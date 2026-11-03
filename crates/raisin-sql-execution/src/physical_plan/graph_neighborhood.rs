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
//! Every limit FAILS the query rather than cutting the result short (a
//! neighbourhood cut short is indistinguishable from a complete one once it
//! reaches the client, which is the failure shape the path cap in
//! `pgq::matching::traversal` already refuses). There are two kinds:
//!
//! * the CALLER's budget, `max_nodes`: it counts only nodes the caller may
//!   read (the start nodes included). Nodes the caller cannot read are walked
//!   through but never charged to it, so moving `max_nodes` until the search
//!   fails measures readable nodes only, which the caller could count anyway;
//! * FIXED server ceilings no query can change: every node visited, readable or
//!   not ([`NeighborhoodParams::max_visited`]), every relation READ from the
//!   per-node indexes and a wall-clock deadline (both in the [`GraphBudget`],
//!   checked after every seek and every visibility check).
//!
//! All of them fail with the same few words and no numbers: no counts, no node
//! ids. The counts are logged at debug level on the server instead.
//!
//! # Visibility
//!
//! Every node the search reaches is checked once, when it is first reached, so
//! the search knows what it may charge to the caller.
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
//! before the next one is loaded. Only node KEYS (and whether each is
//! readable) are held for a whole level; a reported node is loaded again, a
//! chunk at a time, when it is handed out.

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

/// Largest budget a query may name. Only a system or system-admin caller may
/// name one at all; everyone else gets [`DEFAULT_MAX_NODES`]. See
/// `neighborhood_scan`.
pub const MAX_MAX_NODES: usize = 1_000_000;

/// Fixed ceiling on nodes visited, readable or not. Not caller tunable.
pub const MAX_VISITED: usize = 1_000_000;

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
    /// The caller's budget: READABLE nodes visited, the start nodes included.
    pub max_nodes: usize,
    /// Fixed ceiling on every node visited, readable or not.
    pub max_visited: usize,
    /// Deadline and relation ceiling (fixed).
    pub budget: GraphBudget,
}

/// Decides whether the caller may read a node.
///
/// It is asked about every node the search reaches, once when it is reached
/// (start nodes at `depth` 0), and again for a reported node when it is handed
/// out, to fetch the item.
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
    /// Every node reached, readable or not.
    visited: HashSet<NodeKey>,
    /// How many of `visited` the caller may read: what `max_nodes` limits.
    readable_visited: usize,
    /// The level being handed out, `(node, via_count, readable)` sorted by node.
    level: Vec<(NodeKey, u64, bool)>,
    /// How much of `level` has been handed out.
    cursor: usize,
    /// Depth of `level`.
    depth: u32,
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
                level.push((start.clone(), 0, true));
            }
        }
        level.sort();
        let visited: HashSet<NodeKey> = level.iter().map(|(k, _, _)| k.clone()).collect();
        if visited.len() > params.max_nodes || visited.len() > params.max_visited {
            return Err(refuse(Exhausted::Nodes, visited.len(), &budget));
        }
        Ok(Self {
            access,
            gate,
            params,
            budget,
            readable_visited: visited.len(),
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
                let hits = self.report_chunk().await?;
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

    /// Load the next [`CHUNK`] readable nodes of the current level.
    async fn report_chunk(&mut self) -> Result<Vec<Hit<G::Item>>> {
        if self.depth < self.params.min_depth {
            // Walked through, never reported.
            self.cursor = self.level.len();
            return Ok(Vec::new());
        }
        let depth = self.depth;
        let end = (self.cursor + CHUNK).min(self.level.len());
        let mut hits = Vec::new();
        for i in self.cursor..end {
            let (key, via_count, readable) = &self.level[i];
            if !readable {
                continue;
            }
            self.budget
                .check_time()
                .map_err(|why| refuse(why, self.visited.len(), &self.budget))?;
            // Readable when it was reached; a node that has gone since is
            // simply not reported.
            let Some(item) = self.gate.report(&key.0, &key.1, depth).await? else {
                continue;
            };
            hits.push(Hit {
                workspace: key.0.clone(),
                id: key.1.clone(),
                depth,
                via_count: *via_count,
                item,
            });
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
        let frontier = std::mem::take(&mut self.level);
        let next_depth = self.depth + 1;

        // node -> (distinct readable frontier nodes reaching it, index of the
        // last one counted, readable). Frontier nodes are processed one at a
        // time, so comparing against the last index is enough to count each of
        // them once even when two relations (or both directions) join the same
        // pair.
        let mut reached: HashMap<NodeKey, (u64, usize, bool)> = HashMap::new();
        for (idx, (node, _, node_readable)) in frontier.iter().enumerate() {
            let next = match step(self.access, node, p.side, &p.types, &mut self.budget).await {
                Ok(next) => next,
                Err(StepError::Failed(e)) => return Err(e),
                Err(StepError::Stopped(why)) => {
                    return Err(refuse(why, self.visited.len(), &self.budget))
                }
            };
            for next in next {
                if self.visited.contains(&next) {
                    continue;
                }
                if !reached.contains_key(&next) {
                    // First time reached: decide once whether the caller may
                    // read it, which decides what it is charged to.
                    self.budget
                        .check_time()
                        .map_err(|why| refuse(why, self.visited.len(), &self.budget))?;
                    let readable = self
                        .gate
                        .report(&next.0, &next.1, next_depth)
                        .await?
                        .is_some();
                    if readable {
                        self.readable_visited += 1;
                        if self.readable_visited > p.max_nodes {
                            return Err(refuse(Exhausted::Nodes, self.visited.len(), &self.budget));
                        }
                    }
                    if self.visited.len() + reached.len() + 1 > p.max_visited {
                        return Err(refuse(Exhausted::Nodes, self.visited.len(), &self.budget));
                    }
                    reached.insert(next.clone(), (0, usize::MAX, readable));
                }
                if let Some(entry) = reached.get_mut(&next) {
                    if *node_readable && entry.1 != idx {
                        entry.0 += 1;
                        entry.1 = idx;
                    }
                }
            }
        }

        let mut level: Vec<(NodeKey, u64, bool)> = reached
            .into_iter()
            .map(|(k, (n, _, readable))| (k, n, readable))
            .collect();
        level.sort();
        self.visited.extend(level.iter().map(|(k, _, _)| k.clone()));
        self.level = level;
        self.cursor = 0;
        self.depth = next_depth;
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
