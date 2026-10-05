//! Commit-time localized name index work (plan Phase 12).
//!
//! A transaction stages its node records and its overlays in ONE batch, and a
//! batch is not readable before it is written: the record writer's sync saw
//! the overlays as committed before this transaction, and the overlay
//! writer's sync saw the node as committed before it — both outside any lock,
//! so a concurrent commit of the same node may have landed since. So every
//! node this transaction wrote is synced once more here, UNDER THE BRANCH
//! LOCK (every commit of the branch is serialized there), against the
//! transaction's final view (its written node, its written overlays) and the
//! state every earlier commit left: a FULL put into the same batch, whose
//! later puts of the same keys win over the partial-view rows. Uniqueness,
//! when enforced, is checked here too — under the lock, so two siblings
//! claiming one name concurrently cannot both pass — and against the other
//! nodes of THIS commit by their final view (`pending_names`, plan Phase
//! 13c), so two siblings named alike by one transaction cannot both pass
//! either.

use super::super::change_types::{ChangedNodesMap, ChangedTranslationsMap};
use super::super::pending_names::TxPending;
use super::super::RocksDBTransaction;
use crate::localized_name::keys::NameScope;
use crate::localized_name::sync::{sync_node_final, Overrides};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;
use rocksdb::WriteBatch;
use std::collections::BTreeMap;

impl RocksDBTransaction {
    pub(super) fn stage_localized_names(
        &self,
        batch: &mut WriteBatch,
        (tenant_id, repo_id, branch): (&str, &str, &str),
        changed_nodes: &ChangedNodesMap,
        changed_translations: &ChangedTranslationsMap,
        staged_at: &HLC,
    ) -> Result<()> {
        if !crate::localized_name::enabled() {
            return Ok(());
        }
        // One configuration read for the whole commit; uniqueness is checked
        // only where the repository enforces it.
        let Some(cfg) = crate::localized_name::config::load(&self.db, tenant_id, repo_id)? else {
            return Ok(());
        };
        let enforce = cfg.enforce_unique;
        // (workspace, node id) -> revision. Every node written: a node whose
        // record alone changed may still have rows (a move) staged from a
        // view a concurrent commit has since superseded. The sync costs two
        // key probes for a node that never had a translated name.
        let mut targets: BTreeMap<(String, String), HLC> = BTreeMap::new();
        for ((node_id, _), change) in changed_translations {
            targets.insert((change.workspace.clone(), node_id.clone()), change.revision);
        }
        for (node_id, change) in changed_nodes {
            if change.operation == ChangeOperation::Deleted {
                continue;
            }
            targets
                .entry((change.workspace.clone(), node_id.clone()))
                .or_insert(change.revision);
        }
        if targets.is_empty() {
            return Ok(());
        }
        let cache = self
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        for ((workspace, node_id), revision) in targets {
            let key = (workspace.clone(), node_id.clone());
            let node: Option<Node> = match cache.nodes.get(&key) {
                Some(Some(node)) => Some(node.clone()),
                Some(None) => None,
                None => match cache.moved_nodes.get(&key) {
                    Some(node) => Some(node.clone()),
                    None => crate::localized_name::sync::node_at(
                        &self.db,
                        NameScope::new(tenant_id, repo_id, branch, &workspace),
                        &node_id,
                        None,
                    )?
                    .and_then(|(_, node)| node),
                },
            };
            let Some(node) = node else { continue };
            let scope = NameScope::new(tenant_id, repo_id, branch, &workspace);
            let pending = TxPending {
                cache: &cache,
                scope,
                workspace: &workspace,
                staged_at: *staged_at,
            };
            let overrides: Overrides = pending.overrides(&node_id);
            // At the NEWEST state, under this lock — never as of `revision`
            // (the transaction's, allocated at its first write, or a
            // `versionable=false` node's reused one), where a parent created
            // or moved into place since does not resolve.
            let parent = pending.parent_of(&self.db, &node.path)?;
            if enforce {
                // Against stored state AND every other node this commit
                // writes, each judged by its final view (plan Phase 13c).
                crate::localized_name::unique::check_unique_in(
                    &self.db,
                    scope,
                    &node,
                    parent.as_deref(),
                    &revision,
                    &overrides,
                    &pending,
                )?;
            }
            sync_node_final(
                &self.db,
                batch,
                scope,
                &node,
                parent.as_deref(),
                &revision,
                &overrides,
            )?;
        }
        Ok(())
    }
}
