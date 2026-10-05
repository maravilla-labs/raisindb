//! A transaction's uncommitted writes as localized-name sibling uniqueness
//! sees them (plan Phase 13c): [`TxPending`] implements
//! `localized_name::unique::PendingWrites` over the read cache, and the two
//! transaction checkpoints share how a written node's final view is built —
//! the overlay write (at staging, before anything is staged) and the commit
//! (under the branch lock, `commit/localized_names.rs`).

use super::metadata::ReadCache;
use super::RocksDBTransaction;
use crate::localized_name::keys::{NameScope, ROOT_PARENT};
use crate::localized_name::sync::{node_at, parent_path_of, Overrides};
use crate::localized_name::unique::{self, PendingWrites};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleOverlay;

/// One workspace of a transaction's read cache.
pub(super) struct TxPending<'a> {
    pub(super) cache: &'a ReadCache,
    pub(super) scope: NameScope<'a>,
    pub(super) workspace: &'a str,
    /// The revision the transaction stages its overlays at.
    pub(super) staged_at: HLC,
}

impl TxPending<'_> {
    fn key(&self, id: &str) -> (String, String) {
        (self.workspace.to_string(), id.to_string())
    }

    /// The overlays this transaction staged for `id`, by locale (a range of
    /// the ordered cache, not a scan of every staged overlay).
    pub(super) fn overrides(&self, id: &str) -> Overrides {
        self.cache
            .translations_of(self.workspace, id)
            .map(|(locale, overlay)| (locale.clone(), overlay.clone()))
            .collect()
    }

    /// The forward-key parent id of a node at `path` as the commit will
    /// leave it: the workspace root, a parent path this transaction wrote,
    /// else the NEWEST stored path index entry. Never the index as of the
    /// transaction's revision: that is allocated at its first write, so a
    /// parent another commit created (or moved into place) since resolved
    /// to nothing — every stored claim under it skipped by the uniqueness
    /// check, and the workspace's build invalidated by the index writer.
    pub(super) fn parent_of(&self, db: &rocksdb::DB, path: &str) -> Result<Option<String>> {
        let parent_path = parent_path_of(path);
        if parent_path == "/" {
            return Ok(Some(ROOT_PARENT.to_string()));
        }
        if let Some(written) = self
            .cache
            .paths
            .get(&(self.workspace.to_string(), parent_path.clone()))
        {
            return Ok(written.clone());
        }
        let entry = crate::mvcc_read::path_index_entry_in(
            &mut crate::mvcc_read::DbRead(db),
            self.scope.tenant_id,
            self.scope.repo_id,
            self.scope.branch,
            self.workspace,
            &parent_path,
            None,
        )?;
        Ok(entry.and_then(|(_, id)| id))
    }
}

impl PendingWrites for TxPending<'_> {
    fn node(&self, id: &str) -> Option<Option<Node>> {
        let key = self.key(id);
        if let Some(node) = self.cache.nodes.get(&key) {
            return Some(node.clone());
        }
        self.cache.moved_nodes.get(&key).cloned().map(Some)
    }

    fn overlay(&self, id: &str, locale: &str) -> Option<Option<LocaleOverlay>> {
        self.cache
            .translations
            .get(&(
                self.workspace.to_string(),
                id.to_string(),
                locale.to_string(),
            ))
            .cloned()
    }

    fn at_path(&self, path: &str) -> Option<Option<String>> {
        self.cache
            .paths
            .get(&(self.workspace.to_string(), path.to_string()))
            .cloned()
    }

    fn named(&self, locale: &str, name: &str) -> Vec<String> {
        self.cache
            .node_name_hints
            .get(&(
                self.workspace.to_string(),
                locale.to_string(),
                name.to_string(),
            ))
            .map(|ids| ids.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn staged_at(&self) -> HLC {
        self.staged_at
    }
}

impl RocksDBTransaction {
    /// Sibling uniqueness of the overlay `store_translation` is about to
    /// stage: `node_id`'s final view with it, against stored state and
    /// everything else this transaction wrote. A no-op unless the repository
    /// enforces uniqueness on this branch's workspace.
    pub(super) fn check_overlay_unique(
        &self,
        (tenant_id, repo_id, branch): (&str, &str, &str),
        workspace: &str,
        node_id: &str,
        locale: &str,
        overlay: &LocaleOverlay,
        revision: &HLC,
    ) -> Result<()> {
        let scope = NameScope::new(tenant_id, repo_id, branch, workspace);
        if unique::active(&self.db, scope)?.is_none() {
            return Ok(());
        }
        let cache = self
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let pending = TxPending {
            cache: &cache,
            scope,
            workspace,
            staged_at: *revision,
        };
        let node = match pending.node(node_id) {
            Some(Some(node)) => node,
            Some(None) => return Ok(()),
            None => match node_at(&self.db, scope, node_id, None)? {
                Some((_, Some(node))) => node,
                _ => return Ok(()),
            },
        };
        let mut overrides = pending.overrides(node_id);
        overrides.insert(locale.to_string(), Some(overlay.clone()));
        unique::check_unique_in(
            &self.db,
            scope,
            &node,
            pending.parent_of(&self.db, &node.path)?.as_deref(),
            revision,
            &overrides,
            &pending,
        )
    }
}
