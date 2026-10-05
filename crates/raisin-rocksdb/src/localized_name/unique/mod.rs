//! Sibling uniqueness of EFFECTIVE localized names, per
//! `(branch, ws, parent, locale)`.
//!
//! Enforced only when the repository asks for it
//! (`localized_names.enforce_unique`) AND this branch's build of the
//! workspace is `Ready` under the current configuration with ZERO
//! collisions: enforcing on a branch whose index might hold an unseen
//! collision, or is incomplete, would refuse or admit writes arbitrarily.
//! Until then collisions resolve deterministically (the newest claim wins on
//! every node) and the build reports them.
//!
//! A node's effective segment in a locale is its translated name there, or —
//! without one — its canonical name. So a write collides when, in some
//! locale where the node is not hidden, its effective name is:
//!
//! 1. claimed by a sibling (a forward claim probe, every claim verified
//!    through the selector, the way UNIQUE_INDEX is probed) or named so by an
//!    overlay the same transaction staged ([`PendingWrites::named`]);
//! 2. — when it is a TRANSLATED name — the canonical name of a visible
//!    sibling that has no translated name of its own there.
//!
//! (Two canonical names are the path index's business, never this one's.)
//! Every sibling found is judged by its FINAL view: the version the writing
//! transaction leaves where it wrote one ([`pending`]), the stored one
//! otherwise.
//!
//! Called by EVERY LOCAL write path that places a node under a parent or
//! gives it a name (plan Phase 13c lists them): the transaction commit
//! (under the branch lock) and the transaction's overlay write (at staging,
//! so a package install refuses one overlay, not its batch), the repository
//! create, update and translation writes, and the repository move and
//! same-branch tree copy (for the moved or copied root, under its
//! destination parent). Every one of them checks AGAIN under the branch
//! record lock right before its write — the transaction commit in
//! `stage_localized_names`, the repository paths through [`NameCheck`] —
//! so no two writes can both pass against a state neither has written yet.
//! Only names a write CHANGES are refused ([`stored`]). Never by replication apply, merge or cross-branch
//! promotion, which carry decisions another node or branch already made (the
//! `allowed_children` / uniqueness trust model) — collisions those bring in
//! are found by the next build.

mod deferred;
mod pending;
mod stored;

pub(crate) use deferred::NameCheck;
pub(crate) use pending::{NoPending, PendingWrites};

use super::config::{self, NameConfig};
use super::keys::NameScope;
use super::lookup::view::join_path;
use super::rows;
use super::state;
use super::sync::{overlays_at, parent_key, parent_path_of, Overrides};
use crate::indexing::localized_node_names::localized_node_names;
use pending::final_view;
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;
use std::collections::BTreeSet;

/// Refuse (with `Conflict`) a write that gives `node` the same effective
/// name as a sibling in a locale, when enforcement is active.
///
/// `revision` is where the write's overlays (`overrides`) are staged, and
/// ONLY that: it decides whether a newer stored version beats a staged one.
/// Everything else is read at the NEWEST state — the parent, the node's
/// stored overlays, the sibling claims and paths — because that is what the
/// write lands on (under the branch lock at every commit step). A bound at
/// the write's revision judged a transaction that allocated its revision
/// early, or a `versionable=false` rewrite at its reused one, against
/// superseded names and a parent path that no longer resolves.
pub(crate) fn check_unique(
    db: &DB,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
    overrides: &Overrides,
) -> Result<()> {
    check_unique_in(db, scope, node, parent_id, revision, overrides, &NoPending)
}

/// [`check_unique`] for a write inside an uncommitted transaction: `node`
/// and `overrides` are its final view, `pending` everything else the
/// transaction wrote in the workspace.
///
/// Only a name the write CHANGES is refused: a locale whose effective name
/// and parent equal the node's stored ones is skipped. A collision already
/// stored (one replication, merge or promotion brought in — never refused,
/// see the module doc) is not this write's doing, and refusing it wedged
/// every later write of either node, a property-only update included.
pub(crate) fn check_unique_in(
    db: &DB,
    scope: NameScope<'_>,
    node: &Node,
    parent_id: Option<&str>,
    revision: &HLC,
    overrides: &Overrides,
    pending: &dyn PendingWrites,
) -> Result<()> {
    let Some(cfg) = active(db, scope)? else {
        return Ok(());
    };
    let parent_path = parent_path_of(&node.path);
    // `None`: the parent is not stored (the same transaction creates it), so
    // no stored claim can sit under it; staged siblings are still checked.
    let parent = parent_key(db, scope, node, parent_id, &crate::mvcc_read::NEWEST)?;
    let overlays = overlays_at(db, scope, &node.id, None, overrides, revision)?;
    // Supported locales, plus any other locale the node has a name in.
    let locales: BTreeSet<String> = cfg
        .locales
        .iter()
        .cloned()
        .chain(localized_node_names(&overlays, &cfg).into_keys())
        .collect();
    let stored = stored::StoredNames::load(db, scope, &cfg, &node.id, &locales)?;
    for locale in &locales {
        let Some((name, translated)) = stored::effective(&overlays, locale, &cfg, node) else {
            continue;
        };
        if stored
            .as_ref()
            .is_some_and(|s| s.unchanged(&parent_path, locale, &name))
        {
            continue;
        }
        let found = match claimed_by_sibling(
            db,
            scope,
            &cfg,
            pending,
            node,
            parent.as_deref(),
            locale,
            &name,
        )? {
            Some(other) => Some(other),
            None if translated => {
                canonical_sibling(db, scope, &cfg, pending, node, &parent_path, locale, &name)?
            }
            None => None,
        };
        if let Some(other) = found {
            return Err(conflict(&name, locale, &other, &parent_path));
        }
    }
    Ok(())
}

/// The repository's name configuration when uniqueness is enforced on this
/// branch's workspace (`None`: nothing to check).
pub(crate) fn active(db: &DB, scope: NameScope<'_>) -> Result<Option<NameConfig>> {
    if !super::enabled() {
        return Ok(None);
    }
    let Some(cfg) = config::load(db, scope.tenant_id, scope.repo_id)? else {
        return Ok(None);
    };
    Ok((cfg.enforce_unique && enforcing(db, scope, &cfg)?).then_some(cfg))
}

/// [`check_unique`] for a COPY of `source_id`: the copy carries the source's
/// live overlays, which are not its own yet.
pub(crate) fn source_overrides(
    db: &DB,
    scope: NameScope<'_>,
    source_id: &str,
) -> Result<Overrides> {
    Ok(overlays_at(
        db,
        scope,
        source_id,
        None,
        &Overrides::new(),
        &crate::mvcc_read::NEWEST,
    )?
    .into_iter()
    .map(|(locale, overlay)| (locale, Some(overlay)))
    .collect())
}

/// Whether enforcement is active on this branch's workspace: `Ready` under
/// the current configuration with zero collisions.
fn enforcing(db: &DB, scope: NameScope<'_>, cfg: &NameConfig) -> Result<bool> {
    let record = state::read(
        db,
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
    )?;
    Ok(record.as_ref().is_some_and(|s| s.collisions == 0)
        && state::availability(record.as_ref(), &cfg.fingerprint(), None).is_ready())
}

/// Another node whose final view names it `name` in `locale` under `node`'s
/// parent: the stored claims on `(locale, parent, name)` and the overlays
/// the same transaction staged with that name.
#[allow(clippy::too_many_arguments)]
fn claimed_by_sibling(
    db: &DB,
    scope: NameScope<'_>,
    cfg: &NameConfig,
    pending: &dyn PendingWrites,
    node: &Node,
    parent: Option<&str>,
    locale: &str,
    name: &str,
) -> Result<Option<String>> {
    let parent_path = parent_path_of(&node.path);
    let stored = match parent {
        Some(parent) => rows::claims(db, scope, locale, parent, name, None)?,
        None => Vec::new(),
    };
    let chain = [locale.to_string()];
    let mut seen = BTreeSet::new();
    for other in stored
        .into_iter()
        .map(|(_, id)| id)
        .chain(pending.named(locale, name))
    {
        if other == node.id || !seen.insert(other.clone()) {
            continue;
        }
        let Some(view) = final_view(db, scope, pending, &other, &chain)? else {
            continue;
        };
        if view.answers(&parent_path, locale, name, cfg) {
            return Ok(Some(other));
        }
    }
    Ok(None)
}

/// The visible sibling whose CANONICAL name is `name` and which has no
/// translated name of its own in `locale`'s chain (so `name` is its
/// effective segment there).
#[allow(clippy::too_many_arguments)]
fn canonical_sibling(
    db: &DB,
    scope: NameScope<'_>,
    cfg: &NameConfig,
    pending: &dyn PendingWrites,
    node: &Node,
    parent_path: &str,
    locale: &str,
    name: &str,
) -> Result<Option<String>> {
    let path = join_path(parent_path, name);
    let other = match pending.at_path(&path) {
        Some(Some(other)) => other,
        Some(None) => return Ok(None),
        None => match crate::mvcc_read::path_index_entry_in(
            &mut crate::mvcc_read::DbRead(db),
            scope.tenant_id,
            scope.repo_id,
            scope.branch,
            scope.workspace,
            &path,
            None,
        )? {
            Some((_, Some(other))) => other,
            _ => return Ok(None),
        },
    };
    if other == node.id {
        return Ok(None);
    }
    let chain = cfg.fallback_chain(locale);
    let Some(view) = final_view(db, scope, pending, &other, &chain)? else {
        return Ok(None);
    };
    let collides = view.node.path == path && view.visible() && view.segment(&chain, cfg) == name;
    Ok(collides.then_some(other))
}

fn conflict(name: &str, locale: &str, other: &str, parent_path: &str) -> Error {
    Error::Conflict(format!(
        "localized node name '{name}' ({locale}) is already used by sibling {other} under {parent_path}"
    ))
}
