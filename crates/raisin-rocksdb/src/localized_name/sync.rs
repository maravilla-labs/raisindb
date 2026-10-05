//! `sync_node`: the ONE writer of `cf::LOCALIZED_NAME_INDEX` entries.
//!
//! Every write funnel calls into here, inline, in its own `WriteBatch`:
//!
//! - the node record writer (`write_node_record{,_keeping_path}`), which every
//!   create / update / move / reorder / copy / deep create / transaction /
//!   merge / replication / backup-import write of a live node goes through;
//! - the translation version writer (`translation_write::stage_version`), for
//!   every overlay write (repository, transaction, copy, merge, replication);
//! - the delete tombstoner (`tombstones::add_node_tombstones_with_parent`);
//! - the transaction commit (under the branch lock), the copies and merge
//!   apply, which stage a node and its overlays in ONE batch and so call
//!   [`sync_node_final`] once more with the final view: a FULL put that
//!   overwrites whatever the earlier partial-view syncs of the same batch
//!   staged at the same revision (a batch is not readable before it is
//!   written).
//!
//! **Bounded by the incoming revision.** The previous state is the node's
//! reverse rows as of the write's revision (newest at or before it), never the
//! newest rows: an out-of-order replicated write lands between its neighbours
//! instead of overwriting the newest state. Each changed locale gets a forward
//! claim and a reverse row at the revision, and the previous forward claim of
//! THIS node gets a `T` there.
//!
//! **Catch-up.** A write below newer state of the same node (an out-of-order
//! apply, a `versionable=false` in-place rewrite under a newer overlay) also
//! re-derives the node's state as of its newest record and stages it at that
//! revision, against the stored rows plus what this sync just staged, so a
//! HEAD lookup always finds the current segment.
//!
//! **Cheap when there is nothing to index.** A node with no translation
//! version and no index row at all (the common case: a repository without
//! translated names) costs two key probes and nothing else.
//!
//! **Never silently incomplete.** A node with a name to claim whose parent
//! cannot be resolved invalidates its workspace's build (`NotBuilt`, a rebuild
//! requested) instead of leaving a `Ready` index missing it.

use super::catch_up::{self, Newest};
use super::config;
use super::keys::{self, NameScope};
use super::plan::{Planned, Prior};
use super::rows;
use crate::indexing::localized_node_names::localized_node_names;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use rocksdb::{WriteBatch, DB};
use std::collections::{BTreeMap, BTreeSet};

/// Overlays a caller has staged but not written, by locale (`None`: deleted).
/// They replace what the database holds for those locales.
pub(crate) type Overrides = BTreeMap<String, Option<LocaleOverlay>>;

/// Stage the index rows for `node` as written at `revision`, its overlays read
/// from the database. `parent_id`: the record writer's parent id (`None` or
/// `"/"` for a root child); a missing one for a non-root node is resolved
/// from the parent's path.
pub(crate) fn sync_node(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
) -> Result<()> {
    sync(
        db,
        batch,
        scope,
        node,
        parent_id,
        revision,
        &Overrides::new(),
        false,
    )
}

/// The FINAL-view sync of a funnel that staged a node and its overlays in one
/// batch: `overrides` are its staged overlays, and every locale's final state
/// is re-staged at `revision` (see the module doc) — including over the rows
/// a merge's `copy_branch_indexes` replays below it.
pub(crate) fn sync_node_final(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
    overrides: &Overrides,
) -> Result<()> {
    sync(db, batch, scope, node, parent_id, revision, overrides, true)
}

/// [`sync_node_final`] with no staged overlays (merge apply: "full put at M,
/// never a delta").
pub(crate) fn sync_node_full(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
) -> Result<()> {
    sync_node_final(
        db,
        batch,
        scope,
        node,
        parent_id,
        revision,
        &Overrides::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn sync(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
    overrides: &Overrides,
    full: bool,
) -> Result<()> {
    if !super::enabled() {
        return Ok(());
    }
    // Key probes first: a node that never had a translation version nor an
    // index row has nothing to claim and nothing to give up.
    let mut newest = Newest {
        record: None,
        overlay: catch_up::newest_translation_revision(db, scope, &node.id, None)?,
        row: rows::newest_reverse_revision(db, scope, &node.id)?,
    };
    if overrides.is_empty() && newest.overlay.is_none() && newest.row.is_none() {
        return Ok(());
    }
    let Some(cfg) = config::load(db, scope.tenant_id, scope.repo_id)? else {
        return Ok(());
    };
    let overlays = overlays_at(db, scope, &node.id, Some(revision), overrides, revision)?;
    let desired = localized_node_names(&overlays, &cfg);
    let parent = if desired.is_empty() {
        None
    } else {
        match resolve_parent(db, scope, node, parent_id, revision)? {
            Some(parent) => Some(parent),
            None => return Ok(()),
        }
    };
    let mut prior = Prior {
        full,
        ..Prior::default()
    };
    if full {
        // Every locale an earlier partial-view sync of this batch can have
        // staged a row for: the stored overlays and the staged ones.
        let (t, r, b, ws) = (
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
        );
        let stored: BTreeSet<String> =
            crate::translation_read::live_locales(db, t, r, b, ws, &node.id, Some(revision))?
                .into_iter()
                .collect();
        prior.locales = stored
            .into_iter()
            .chain(overrides.keys().cloned())
            .collect();
    }
    let main: Planned = super::plan::apply(
        db,
        batch,
        scope,
        &node.id,
        parent.as_deref(),
        &desired,
        revision,
        &prior,
    )?;
    newest.record = catch_up::newest_record_revision(db, scope, &node.id)?;
    catch_up::catch_up(
        db,
        batch,
        scope,
        node,
        parent.as_deref(),
        revision,
        overrides,
        &cfg,
        newest,
        &main,
        full,
    )
}

/// The forward-key parent of `node` at `revision`; when it cannot be
/// resolved, the workspace's build is invalidated (the node has a name to
/// claim, so a `Ready` index without it would 404 it) and `None` returned.
pub(crate) fn resolve_parent(
    db: &DB,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
) -> Result<Option<String>> {
    if let Some(parent) = parent_key(db, scope, node, parent_id, revision)? {
        return Ok(Some(parent));
    }
    tracing::warn!(
        node_id = %node.id,
        path = %node.path,
        "localized name index: the parent of a written node is unknown; \
         workspace build invalidated"
    );
    super::state::invalidate_workspace(
        db,
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
    )?;
    super::auto::request_build(scope.tenant_id, scope.repo_id, scope.branch);
    Ok(None)
}

/// After an overlay write of `node_id` in `locale` at `revision` (`None`:
/// the translation was deleted), re-derive the node's segments with it. The
/// node as of `revision` comes from the database; a node not committed yet
/// (created in the same batch) is synced by its own writer.
pub(crate) fn sync_after_overlay(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node_id: &str,
    locale: &str,
    overlay: Option<&LocaleOverlay>,
    revision: &HLC,
) -> Result<()> {
    if !super::enabled() {
        return Ok(());
    }
    let Some((_, Some(node))) = node_at(db, scope, node_id, Some(revision))? else {
        return Ok(());
    };
    let overrides = Overrides::from([(locale.to_string(), overlay.cloned())]);
    sync(db, batch, scope, &node, None, revision, &overrides, false)
}

/// A delete of `node_id` at `revision`: its live claims and reverse rows as
/// of the delete get `T` there. (A lookup also checks the node is live, so a
/// delete applied without this — a checkpoint, an old peer — is never served.)
pub(crate) fn stage_node_deleted(
    db: &DB,
    batch: &mut WriteBatch,
    scope: NameScope<'_>,
    node_id: &str,
    revision: &HLC,
) -> Result<()> {
    if !super::enabled() {
        return Ok(());
    }
    let cf = cf_handle(db, cf::LOCALIZED_NAME_INDEX)?;
    for (locale, (_, segment)) in rows::reverse_rows(db, scope, node_id, Some(revision))? {
        let Some(segment) = segment else { continue };
        let key = keys::forward_key(
            scope,
            &locale,
            &segment.parent_id,
            &segment.name,
            node_id,
            revision,
        );
        batch.put_cf(cf, key, crate::keys::TOMBSTONE_VALUE);
        batch.put_cf(
            cf,
            keys::reverse_key(scope, node_id, &locale, revision),
            crate::keys::TOMBSTONE_VALUE,
        );
    }
    Ok(())
}

pub(crate) use super::plan::plan_rows;
pub(crate) use super::reads::{node_at, overlays_at, parent_key, parent_path_of};
