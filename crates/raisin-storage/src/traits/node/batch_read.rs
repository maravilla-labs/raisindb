// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Batched snapshot reads: [`super::NodeRepository::get_many_for_read`].
//!
//! A read-heavy statement (RESOLVE's frontier, an index scan's candidate ids)
//! used to await one `get` per node. The batched read hands the storage a
//! whole level or chunk at once, so a backend can amortize iterator setup and
//! keep the work off the async workers.
//!
//! The answer for every item is EXACTLY what `get` (an id) or `get_by_path`
//! (a path) would return at the same revision — the equivalence the property
//! tests pin — except that `has_children` is only populated when
//! [`ReadOpts::has_children`] asks for it (it is `None` otherwise).

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::any::Any;
use std::sync::Arc;

use super::NodeRepository;
use crate::scope::{BranchScope, StorageScope};

/// How an item names its node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeLocator {
    /// A node id — answered like `get`.
    Id(String),
    /// A node path — answered like `get_by_path` (PATH_INDEX at the revision).
    Path(String),
}

/// One node to read: its workspace and locator.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BatchReadItem {
    pub workspace: String,
    pub locator: NodeLocator,
}

impl BatchReadItem {
    /// An item naming the node by id.
    pub fn id(workspace: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            locator: NodeLocator::Id(id.into()),
        }
    }

    /// An item naming the node by path.
    pub fn path(workspace: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            workspace: workspace.into(),
            locator: NodeLocator::Path(path.into()),
        }
    }
}

/// Which properties a batched read decodes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PropertiesRead {
    /// All of them — what `get` returns.
    #[default]
    Load,
    /// None: the property map comes back empty (a projected decode for a
    /// caller that never looks at it).
    Skip,
}

/// What a batched read populates beyond the stored record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadOpts {
    pub properties: PropertiesRead,
    /// Run the `has_children` existence probe per node (what `get` does). Off,
    /// `has_children` is `None`: the SQL row and RESOLVE's inlined node never
    /// carry it, so neither caller pays for it.
    pub has_children: bool,
}

/// A storage-level consistent view held for a whole statement.
///
/// The HLC bound a read takes does not protect against an IN-PLACE overwrite
/// at a revision at or below it (`versionable=false` reuses its revision), so a
/// statement reading in several chunks could see such a node before the write
/// in one chunk and after it in the next. A backend that can pin a point-in-time
/// view (a RocksDB snapshot) returns one from
/// [`super::NodeRepository::open_read_snapshot`]; every batched read of the
/// statement then goes through it.
///
/// Opaque on purpose: only the backend that opened it can read it, and any
/// other backend ignores it. Dropping the last clone releases the view.
#[derive(Clone)]
pub struct ReadSnapshot {
    inner: Arc<dyn Any + Send + Sync>,
}

impl ReadSnapshot {
    /// Wrap a backend's view.
    pub fn new<T: Any + Send + Sync>(view: T) -> Self {
        Self {
            inner: Arc::new(view),
        }
    }

    /// The backend's view, when this snapshot was opened by that backend.
    pub fn downcast<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.inner.downcast_ref::<T>()
    }

    /// The shared view itself, for a backend that moves it to another thread.
    pub fn downcast_arc<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.inner.clone().downcast::<T>().ok()
    }
}

impl std::fmt::Debug for ReadSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReadSnapshot(..)")
    }
}

/// The reference behaviour: one `get` / `get_by_path` per item, in order.
///
/// The trait's default (so a backend without a batched reader — the deprecated
/// memory backend — stays correct), and the per-row fallback the SQL engine
/// takes under `sql.batched_fetch = false`.
pub async fn get_many_by_loop<R: NodeRepository + ?Sized>(
    repo: &R,
    scope: BranchScope<'_>,
    items: &[BatchReadItem],
    at: &HLC,
    opts: &ReadOpts,
) -> Result<Vec<Option<Node>>> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let node_scope = StorageScope::new(
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            &item.workspace,
        );
        let node = match &item.locator {
            NodeLocator::Id(id) => repo.get(node_scope, id, Some(at)).await?,
            NodeLocator::Path(path) => repo.get_by_path(node_scope, path, Some(at)).await?,
        };
        out.push(node.map(|node| shape(node, opts)));
    }
    Ok(out)
}

/// The default [`NodeRepository::get_for_read`]: `get` / `get_by_path`,
/// then `opts` applied to the answer.
pub async fn get_one_by_loop<R: NodeRepository + ?Sized>(
    repo: &R,
    scope: StorageScope<'_>,
    locator: &NodeLocator,
    max_revision: Option<&HLC>,
    opts: &ReadOpts,
) -> Result<Option<Node>> {
    let node = match locator {
        NodeLocator::Id(id) => repo.get(scope, id, max_revision).await?,
        NodeLocator::Path(path) => repo.get_by_path(scope, path, max_revision).await?,
    };
    Ok(node.map(|node| shape(node, opts)))
}

/// Apply `opts` to a node `get` returned.
fn shape(mut node: Node, opts: &ReadOpts) -> Node {
    if !opts.has_children {
        node.has_children = None;
    }
    if opts.properties == PropertiesRead::Skip {
        node.properties.clear();
    }
    node
}
