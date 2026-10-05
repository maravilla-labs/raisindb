//! Paths and names of a known node in a locale (alternates, `__node_name`,
//! `__localized_path`): the same selector and visibility rule as the lookup.

use super::view::NodeView;
use super::{LocalizedLookup, Resolution, ServedBy};
use crate::localized_name::config;
use crate::localized_name::keys::NameScope;
use crate::localized_name::lookup::view::join_path;
use crate::mvcc_read::VersionedRead;
use raisin_error::Result;
use raisin_hlc::HLC;

impl LocalizedLookup<'_> {
    /// A path in the default language: the canonical path itself.
    pub(super) fn resolve_canonical(
        &self,
        src: &mut impl VersionedRead,
        scope: NameScope<'_>,
        chain: &[String],
        path: &str,
        bound: Option<&HLC>,
    ) -> Result<Option<Resolution>> {
        Ok(self
            .by_path(src, scope, chain, path, bound)?
            .and_then(|view| {
                view.visible().then(|| Resolution {
                    node_id: view.node.id.clone(),
                    canonical_path: view.node.path.clone(),
                    canonical_localized_path: view.node.path.clone(),
                    redirect: view.node.path != path,
                    served_by: ServedBy::DefaultLanguage,
                })
            }))
    }

    /// The canonical localized path of `node_id` in `locale` at `bound` —
    /// `None` when the node or an ancestor is missing or hidden there.
    pub fn localized_path(
        &self,
        scope: NameScope<'_>,
        node_id: &str,
        locale: &str,
        bound: Option<&HLC>,
    ) -> Result<Option<String>> {
        let db = self.nodes.db_handle();
        // One snapshot-pinned iterator per column family for the whole walk,
        // config included, as the lookup (plan Phase 13d).
        let snapshot = db.snapshot();
        let mut src = crate::mvcc_read::SnapshotRead::new(db, &snapshot);
        let Some(cfg) = config::load_in(&mut src, scope.tenant_id, scope.repo_id)? else {
            return Ok(None);
        };
        let chain = cfg.fallback_chain(locale);
        let Some(target) = NodeView::load_in(&mut src, scope, node_id, &chain, bound)? else {
            return Ok(None);
        };
        let names: Vec<String> = target
            .node
            .path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let mut segments = Vec::with_capacity(names.len());
        let mut path = "/".to_string();
        for name in &names {
            path = join_path(&path, name);
            let Some(view) = self.by_path(&mut src, scope, &chain, &path, bound)? else {
                return Ok(None);
            };
            if !view.visible() {
                return Ok(None);
            }
            segments.push(view.segment(&chain, &cfg));
        }
        Ok(Some(format!("/{}", segments.join("/"))))
    }

    /// The node's own name in `locale` (first chain locale with one) —
    /// `None` when it has none, or is missing or hidden there.
    pub fn node_name(
        &self,
        scope: NameScope<'_>,
        node_id: &str,
        locale: &str,
        bound: Option<&HLC>,
    ) -> Result<Option<String>> {
        let db = self.nodes.db_handle();
        let Some(cfg) = config::load(db, scope.tenant_id, scope.repo_id)? else {
            return Ok(None);
        };
        let chain = cfg.fallback_chain(locale);
        let Some(view) = NodeView::load(db, scope, node_id, &chain, bound)? else {
            return Ok(None);
        };
        if !view.visible() {
            return Ok(None);
        }
        Ok(chain.iter().find_map(|l| match view.name_in(l, &cfg) {
            crate::indexing::localized_node_names::NameIn::Name(s) => Some(s),
            _ => None,
        }))
    }
}
