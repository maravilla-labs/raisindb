//! The catch-up half of `sync_node`: a write below newer state of the same
//! node re-derives the node's state as of its newest record (see `sync`).
//!
//! It diffs against the stored rows as of the newest revision OVERLAID with
//! the rows the main plan just staged at the write's revision: those are not
//! readable yet, and in an A-B-A out-of-order apply (r0 = A, r1 = B, r2 = A,
//! r2 applied before r1) they are exactly what differs — the stored rows as of
//! r2 still say A, so a diff against them alone would leave r1's B as HEAD.

use super::config::NameConfig;
use super::keys::NameScope;
use super::plan::{apply, Planned, Prior};
use super::sync::{overlays_at, resolve_parent, Overrides};
use crate::indexing::localized_node_names::localized_node_names;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

/// The newest revisions of a node's three inputs, from keys alone.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Newest {
    pub(super) record: Option<HLC>,
    pub(super) overlay: Option<HLC>,
    pub(super) row: Option<HLC>,
}

impl Newest {
    pub(super) fn top(&self) -> Option<HLC> {
        [self.record, self.overlay, self.row]
            .into_iter()
            .flatten()
            .max()
    }
}

/// When the node has records newer than `revision`, stage its state as of
/// the newest of them at that revision (see the module doc).
#[allow(clippy::too_many_arguments)]
pub(super) fn catch_up(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node: &Node,
    parent: Option<&str>,
    revision: &HLC,
    overrides: &Overrides,
    cfg: &NameConfig,
    newest: Newest,
    main: &Planned,
    full: bool,
) -> Result<()> {
    let Some(top) = newest.top() else {
        return Ok(());
    };
    if top <= *revision {
        return Ok(());
    }
    // The node as of `top`: only loaded when a newer record exists.
    let (state_node, state_parent) = match newest.record {
        Some(rev) if rev > *revision => {
            match super::sync::node_at(db, scope, &node.id, None)? {
                Some((_, Some(stored))) => (stored, None),
                // Deleted above this write: the delete staged its own tombstones.
                Some((_, None)) => return Ok(()),
                None => (node.clone(), parent.map(str::to_string)),
            }
        }
        _ => (node.clone(), parent.map(str::to_string)),
    };
    let overlays = overlays_at(db, scope, &node.id, None, overrides, revision)?;
    let desired = localized_node_names(&overlays, cfg);
    let state_parent = if desired.is_empty() {
        None
    } else {
        match state_parent {
            Some(p) => Some(p),
            None => match resolve_parent(db, scope, &state_node, None, &top)? {
                Some(p) => Some(p),
                None => return Ok(()),
            },
        }
    };
    let prior = Prior {
        full,
        locales: main.staged.keys().cloned().collect(),
        staged: main.staged.clone(),
    };
    apply(
        db,
        batch,
        scope,
        &node.id,
        state_parent.as_deref(),
        &desired,
        &top,
        &prior,
    )?;
    Ok(())
}

/// The newest stored `TRANSLATION_DATA` revision of the node, any locale,
/// tombstones included, at or before `bound` (`None`: newest at all).
pub(crate) fn newest_translation_revision(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    bound: Option<&HLC>,
) -> Result<Option<HLC>> {
    let prefix = crate::translation_read::node_prefix(
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        node_id,
    );
    let mut newest: Option<HLC> = None;
    crate::translation_read::for_each_newest(
        db,
        cf::TRANSLATION_DATA,
        &prefix,
        1,
        bound,
        |_, rev, _| {
            newest = Some(newest.map_or(rev, |n| n.max(rev)));
        },
    )?;
    Ok(newest)
}

/// The newest record revision of the node (blob or `NODE_PATH`), keys only.
pub(crate) fn newest_record_revision(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
) -> Result<Option<HLC>> {
    crate::mvcc_read::node_record_revision_in(
        &mut crate::mvcc_read::DbRead(db),
        crate::mvcc_read::NodeScope {
            tenant_id: scope.tenant_id,
            repo_id: scope.repo_id,
            branch: scope.branch,
            workspace: scope.workspace,
            node_id,
        },
        &crate::mvcc_read::NEWEST,
    )
}
