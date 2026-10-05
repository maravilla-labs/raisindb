//! The reads every localized-name writer and reader shares: a node as of a
//! revision, its forward-key parent, its live overlays.

use super::keys::{NameScope, ROOT_PARENT};
use super::sync::Overrides;
use crate::repositories::nodes::PropertiesMode;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;
use rocksdb::DB;
use std::collections::BTreeMap;

/// `node_id` as a read at `at` sees it (`None`: newest) — its newest version
/// at or before `at`, with its path materialized AS OF `at` (an ancestor
/// moved since the blob was written still moves it), and `None` for a delete
/// tombstone. Every localized-name reader of a node goes through this.
pub(crate) fn node_at(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    at: Option<&HLC>,
) -> Result<Option<(HLC, Option<Node>)>> {
    node_at_as(
        &mut crate::mvcc_read::DbRead(db),
        scope,
        node_id,
        at,
        PropertiesMode::Load,
    )
}

/// [`node_at`] without decoding the property map — what a lookup needs to
/// verify a candidate (id, name, path; overlays are read separately). The
/// property decode was ~40 % of a localized path lookup (plan Phase 13b).
pub(crate) fn node_head_at(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    at: Option<&HLC>,
) -> Result<Option<(HLC, Option<Node>)>> {
    node_at_as(
        &mut crate::mvcc_read::DbRead(db),
        scope,
        node_id,
        at,
        PropertiesMode::Skip,
    )
}

/// [`node_head_at`] through a read source (a lookup's pinned iterators).
pub(crate) fn node_head_at_in(
    src: &mut impl crate::mvcc_read::VersionedRead,
    scope: NameScope<'_>,
    node_id: &str,
    at: Option<&HLC>,
) -> Result<Option<(HLC, Option<Node>)>> {
    node_at_as(src, scope, node_id, at, PropertiesMode::Skip)
}

fn node_at_as(
    src: &mut impl crate::mvcc_read::VersionedRead,
    scope: NameScope<'_>,
    node_id: &str,
    at: Option<&HLC>,
    mode: PropertiesMode,
) -> Result<Option<(HLC, Option<Node>)>> {
    let bound = at.copied().unwrap_or(crate::mvcc_read::NEWEST);
    let found = crate::mvcc_read::node_version_in(
        src,
        crate::mvcc_read::NodeScope {
            tenant_id: scope.tenant_id,
            repo_id: scope.repo_id,
            branch: scope.branch,
            workspace: scope.workspace,
            node_id,
        },
        &bound,
        mode,
    )?;
    Ok(found.map(|(rev, node)| {
        (
            rev,
            node.map(|mut n| {
                n.workspace = Some(scope.workspace.to_string());
                n
            }),
        )
    }))
}

/// The forward-key parent of `node`: `/` for a root child, else the record
/// writer's id, else the id `PATH_INDEX` gives the parent path at `revision`.
pub(crate) fn parent_key(
    db: &DB,
    scope: NameScope<'_>,
    node: &Node,
    given: Option<&str>,
    revision: &HLC,
) -> Result<Option<String>> {
    let parent_path = parent_path_of(&node.path);
    if parent_path == "/" {
        return Ok(Some(ROOT_PARENT.to_string()));
    }
    if let Some(given) = given.filter(|g| !g.is_empty() && *g != ROOT_PARENT) {
        return Ok(Some(given.to_string()));
    }
    let entry = crate::mvcc_read::path_index_entry_in(
        &mut crate::mvcc_read::DbRead(db),
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        &parent_path,
        Some(revision),
    )?;
    Ok(entry.and_then(|(_, id)| id))
}

/// `/a/b` -> `/a`; `/a` -> `/`.
pub(crate) fn parent_path_of(path: &str) -> String {
    match path.trim_end_matches('/').rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => path[..i].to_string(),
    }
}

/// The node's live overlays as of `bound`, with `overrides` (staged at
/// `override_rev`) replacing a locale unless the database holds a NEWER
/// version of it than the override.
pub(crate) fn overlays_at(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    bound: Option<&HLC>,
    overrides: &Overrides,
    override_rev: &HLC,
) -> Result<BTreeMap<String, LocaleOverlay>> {
    let (t, r, b, ws) = (
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
    );
    let mut out = BTreeMap::new();
    for locale in crate::translation_read::live_locales(db, t, r, b, ws, node_id, bound)? {
        if overrides.contains_key(&locale) {
            continue;
        }
        if let Some(overlay) =
            crate::translation_read::read_overlay(db, t, r, b, ws, node_id, &locale, bound)?
        {
            out.insert(locale, overlay);
        }
    }
    for (locale, overlay) in overrides {
        if let Some(overlay) =
            overlay_over(db, scope, node_id, locale, bound, overlay, override_rev)?
        {
            out.insert(locale.clone(), overlay);
        }
    }
    Ok(out)
}

/// One locale of `node_id` as a reader at `bound` sees it with `staged`
/// (staged at `staged_rev`; `None`: deleted) in front of the database: the
/// stored version wins only when it is NEWER than the staged one. THE
/// precedence rule — [`overlays_at`] and the uniqueness check's view of a
/// sibling (`unique::pending::final_view`) both go through it.
pub(crate) fn overlay_over(
    db: &DB,
    scope: NameScope<'_>,
    node_id: &str,
    locale: &str,
    bound: Option<&HLC>,
    staged: &Option<LocaleOverlay>,
    staged_rev: &HLC,
) -> Result<Option<LocaleOverlay>> {
    let stored = crate::translation_read::read_version(
        db,
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        node_id,
        locale,
        bound,
    )?;
    Ok(match stored {
        Some(version) if version.revision > *staged_rev => version.overlay,
        _ => staged.clone(),
    })
}
