//! Choosing the per-node index over the global one.
//!
//! [`super::seek`] says which pattern variables the `WHERE` clause pins to a
//! known node. This module turns such a pin into concrete `(workspace, id)`
//! nodes and reads their relations from the per-node indexes. Every function
//! here either returns exactly the relations the global scan would have fed the
//! matcher (restricted to the ones that can satisfy the pin), or declines with
//! `None`, in which case the caller scans globally as it always did.

use raisin_sql::ast::{Direction, NodePattern};

use super::seek::{NodeSeek, SeekHints};
use crate::physical_plan::executor::ExecutionError;
use crate::physical_plan::graph_access::{
    resolve_ids, resolve_paths, RelationAccess, SeekNode, Side,
};

type Result<T> = std::result::Result<T, ExecutionError>;

/// Which end of a `(left)-[rel]->(right)` pattern is pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    Left,
    Right,
}

/// How many values a seek names; the pin with fewer values is the cheaper start.
fn seek_size(seek: &NodeSeek) -> usize {
    match (&seek.ids, &seek.paths) {
        (Some(ids), _) => ids.len(),
        (None, Some(paths)) => paths.len(),
        (None, None) => usize::MAX,
    }
}

/// The more selective pinned end of a hop, if either is pinned.
pub(super) fn pinned_end<'h>(
    hints: &'h SeekHints,
    left: &NodePattern,
    right: &NodePattern,
) -> Option<(End, &'h NodeSeek)> {
    let l = hints.seek_for(left.variable.as_ref());
    let r = hints.seek_for(right.variable.as_ref());
    match (l, r) {
        (Some(l), Some(r)) if seek_size(r) < seek_size(l) => Some((End::Right, r)),
        (Some(l), _) => Some((End::Left, l)),
        (None, Some(r)) => Some((End::Right, r)),
        (None, None) => None,
    }
}

/// Which side of the pinned node's relations the hop can use.
///
/// `(a)-[r]->(b)` with `a` pinned reads `a`'s outgoing relations; with `b`
/// pinned, its incoming ones. `<-` swaps that, and an undirected hop can use
/// both.
pub(super) fn side_for(end: End, direction: Direction) -> Side {
    match (end, direction) {
        (_, Direction::Any) => Side::Both,
        (End::Left, Direction::Right) | (End::Right, Direction::Left) => Side::Out,
        (End::Left, Direction::Left) | (End::Right, Direction::Right) => Side::In,
    }
}

/// Resolve a pin to nodes.
///
/// * `Ok(Some(vec![]))`: the pin is contradictory (`a.id = 'x' AND a.id = 'y'`),
///   so no relation can match, and saying so costs nothing.
/// * `Ok(Some(nodes))`: read these nodes' relations.
/// * `Ok(None)`: the pin names no node that exists. A relation can in principle
///   outlive its node, and the global scan would still return it, so the caller
///   scans globally to stay exact rather than answering from a guess.
pub(super) async fn resolve_seek<A: RelationAccess>(
    access: &A,
    seek: &NodeSeek,
) -> Result<Option<Vec<SeekNode>>> {
    let workspaces = seek.workspaces.as_deref();
    let nodes = match (&seek.ids, &seek.paths) {
        (Some(ids), _) => {
            if ids.is_empty() {
                return Ok(Some(vec![]));
            }
            resolve_ids(access, ids, workspaces).await?
        }
        (None, Some(paths)) => {
            if paths.is_empty() {
                return Ok(Some(vec![]));
            }
            resolve_paths(access, paths, workspaces).await?
        }
        (None, None) => return Ok(None),
    };
    if nodes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(nodes))
    }
}
