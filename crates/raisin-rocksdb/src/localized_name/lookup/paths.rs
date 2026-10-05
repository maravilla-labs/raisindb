//! Paths and names of a known node in a locale (alternates, `__node_name`,
//! `__localized_path`): the same selector and visibility rule as the lookup.

use super::view::NodeView;
use super::{LocalizedLookup, Resolution, ServedBy};
use crate::localized_name::keys::NameScope;
use crate::mvcc_read::VersionedRead;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::localized::LocalizedNameSession;

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
        Ok(self
            .session(scope, locale, bound)?
            .names(&[node_id], false, true)?
            .pop()
            .and_then(|names| names.localized_path))
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
        Ok(self
            .session(scope, locale, bound)?
            .names(&[node_id], true, false)?
            .pop()
            .and_then(|names| names.node_name))
    }

    /// A name session of `(scope, locale, bound)` (`session.rs`).
    pub fn session(
        &self,
        scope: NameScope<'_>,
        locale: &str,
        bound: Option<&HLC>,
    ) -> Result<super::session::NameSession> {
        super::session::NameSession::open(self.nodes.db_handle().clone(), scope, locale, bound)
    }
}
