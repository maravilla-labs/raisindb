//! Staging one operation into an open transaction: resolving which node an item
//! maps onto, the etag skip-write, the foreign-node guard, and the in-chunk
//! unique-constraint guard.

use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalContext;
use std::collections::{HashMap, HashSet};

use super::index::{MountScope, SyncIndex, VirtualNodeRef};
use super::node_paths::join_path;
use super::ops::BatchOp;
use super::stamp::watched_subset;
use super::write_view::{carried_pushed_state, write_view_of};
use super::RocksDbMaterializer;
use crate::jobs::handlers::virtual_mount_sync::config::build_properties;

/// What staging one operation did.
pub(super) enum Staged {
    Written,
    Deleted,
    Skipped,
    /// Reserved metadata amended on a node that was already there — the write
    /// drain's stamp-back. Distinct from [`Self::Written`] because the failure
    /// budget and the console's `written` count both mean "an item the provider
    /// reported was materialized", which a stamp is not.
    Stamped,
    /// Held back for a single-item write (an in-chunk unique collision).
    Deferred,
}

/// An index update held until the transaction commits.
pub(super) enum IndexMutation {
    Upsert(VirtualNodeRef),
    Delete { external_id: String },
}

impl IndexMutation {
    pub(super) fn apply(self, index: &mut SyncIndex) {
        match self {
            IndexMutation::Upsert(node_ref) => index.record_upsert(node_ref),
            IndexMutation::Delete { external_id } => index.record_delete(&external_id),
        }
    }
}

impl RocksDbMaterializer {
    /// Stage one operation into the open transaction.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn stage_op(
        &self,
        tx: &dyn TransactionalContext,
        scope: &MountScope,
        index: &SyncIndex,
        op: &BatchOp,
        pending: &mut Vec<IndexMutation>,
        unique_seen: &mut HashSet<(String, String, String)>,
        unique_props: &mut HashMap<String, Vec<String>>,
        defer_unique_collisions: bool,
    ) -> Result<Staged> {
        let (rel_path, mapped, virt) = match op {
            BatchOp::Delete { external_id } => {
                let Some(existing) = index.by_external(external_id) else {
                    return Ok(Staged::Skipped);
                };
                let node_id = existing.id.clone();

                // CASCADE. `TransactionalContext::delete_node` deliberately does
                // not descend (its own doc comment says so), so a mail deleted
                // upstream would leave its `raisin:Asset` attachment children
                // behind: mount-owned, orphaned under a tombstoned parent path,
                // never seen by a full walk again — and therefore deleted only
                // by a reconcile that would first have to notice them. They are
                // removed here, before the parent, so a failure part-way leaves
                // the parent alive and the next run retries the whole subtree
                // rather than stranding it.
                //
                // A child already gone is NOT an error. Both the parent and the
                // child carry an `__external_id`, so a full walk that reconciles
                // the message away stages a `Delete` for each of them — and the
                // batch's order is a HashMap iteration order, so the child's own
                // op may already have run. The index is only updated after the
                // transaction commits, so it still reports the child as live.
                // Propagating that `NotFound` aborted the PARENT's delete, and
                // the message survived a reconcile that had just deleted its
                // attachment: a mail with no body left, on roughly half of runs.
                for child_external_id in index.child_external_ids(external_id) {
                    let Some(child) = index.by_external(&child_external_id) else {
                        continue;
                    };
                    match tx.delete_node(&scope.workspace, &child.id).await {
                        Ok(()) => {}
                        Err(raisin_error::Error::NotFound(_)) => continue,
                        Err(e) => return Err(e),
                    }
                    pending.push(IndexMutation::Delete {
                        external_id: child_external_id,
                    });
                }

                tx.delete_node(&scope.workspace, &node_id).await?;
                pending.push(IndexMutation::Delete {
                    external_id: external_id.clone(),
                });
                return Ok(Staged::Deleted);
            }
            BatchOp::StampVirtual {
                node_id,
                external_id,
                etag,
                synced_at,
                pushed_state,
                merged,
                adopt,
                node_bytes: _,
            } => {
                return self
                    .stage_stamp(
                        tx,
                        scope,
                        pending,
                        node_id,
                        external_id,
                        etag.as_deref(),
                        synced_at,
                        pushed_state.as_ref(),
                        merged.as_ref(),
                        *adopt,
                    )
                    .await;
            }
            BatchOp::Upsert {
                rel_path,
                mapped,
                virt,
            } => (rel_path, mapped, virt),
        };

        let new_path = join_path(&scope.mount_path, rel_path);

        // 1. Match by __external_id within the mount subtree (survives renames).
        let (id, path) = match index.by_external(&virt.external_id) {
            Some(existing) => {
                // Etag skip-write: unchanged item → no revision churn. Bypassed
                // by a remap, which exists precisely to re-apply a mapper whose
                // output changed while the provider's item did not.
                if !scope.force_rewrite && virt.etag.is_some() && existing.etag == virt.etag {
                    return Ok(Staged::Skipped);
                }
                // Update in place, preserving id + current path (avoids dupes on
                // rename; a provider-side rename updates this node). On a remap
                // the relocation already happened in `remap_moves`, so this path
                // IS the destination.
                (existing.id.clone(), existing.path.clone())
            }
            None => {
                // 2. Fall back to a path match. A foreign (non-mount) node sitting
                // at the target path must not be clobbered.
                match index.at_path(&new_path) {
                    Some(entry) if !entry.mount_owned => {
                        tracing::warn!(
                            mount_id = %scope.mount_id,
                            path = %new_path,
                            "virtual mount upsert: foreign node occupies target path, skipping"
                        );
                        return Ok(Staged::Skipped);
                    }
                    Some(entry) => {
                        if virt.etag.is_some() && entry.etag == virt.etag {
                            return Ok(Staged::Skipped);
                        }
                        // A mount-owned entry without a known id can only come
                        // from an ancestor folder recorded this run; `upsert_node`
                        // matches by PATH, so a fresh id is harmless — the write
                        // still lands on the existing node.
                        (
                            entry.id.clone().unwrap_or_else(|| nanoid::nanoid!()),
                            new_path.clone(),
                        )
                    }
                    None => (nanoid::nanoid!(), new_path.clone()),
                }
            }
        };

        let name = mapped
            .name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or("item").to_string());

        // The item the provider just reported IS the pushed state: seeding
        // `__pushed_state` from the mapper's own output is what makes a REMOTE
        // change converge on arrival instead of looking like a local edit and
        // being pushed straight back.
        //
        // `watched_subset` answers `None` only when this mount watches NOTHING,
        // i.e. the engine has nothing to say about the baseline — which is not
        // the same as "there is no baseline". An upsert rebuilds the property
        // map from scratch, so leaving it out there DELETES a baseline an
        // earlier `state_only` run recorded; flipping `mode` to `off` for one
        // run would strip every node the delta touched and re-arm the
        // never-pushed-a-local-edit failure on re-enable. Carry the stored one
        // forward instead. (An empty map from `watched_subset` is a real answer
        // — "watching, none of these fields reported" — and must NOT fall
        // through to the carry.)
        let carried = index
            .by_external(&virt.external_id)
            .and_then(|existing| existing.pushed_state().cloned());
        let pushed_state = watched_subset(&mapped.properties, &scope.watched_fields).or(carried);

        let node = Node {
            id,
            node_type: mapped.node_type.clone(),
            name,
            path: path.clone(),
            workspace: Some(scope.workspace.clone()),
            properties: build_properties(&mapped.properties, virt, pushed_state.as_ref()),
            ..Default::default()
        };

        // In-chunk unique guard (see `apply_chunk`). Only meaningful while more
        // than one item shares the transaction, hence the `defer_unique_collisions`
        // flag: a replay of one item must actually write it.
        if defer_unique_collisions
            && self
                .collides_on_unique(scope, &node, unique_seen, unique_props)
                .await?
        {
            tracing::debug!(
                mount_id = %scope.mount_id,
                external_id = %virt.external_id,
                "unique property collides within the batch; deferring to a single-item write"
            );
            return Ok(Staged::Deferred);
        }

        // Deep upsert auto-creates any missing parent folders.
        tx.upsert_deep_node(&scope.workspace, &node, "raisin:Folder")
            .await?;

        let write_view = write_view_of(&node, &scope.watched_fields);
        let carried = carried_pushed_state(&node, &write_view);
        pending.push(IndexMutation::Upsert(VirtualNodeRef {
            id: node.id,
            path,
            external_id: virt.external_id.clone(),
            etag: virt.etag.clone(),
            synced_secs: chrono::DateTime::parse_from_rfc3339(&virt.synced_at)
                .ok()
                .map(|d| d.timestamp()),
            pushed_state: carried,
            write_view,
        }));
        Ok(Staged::Written)
    }

    /// Whether `node` reuses a `unique: true` property value already claimed by
    /// an earlier item in this chunk. Records the values either way.
    async fn collides_on_unique(
        &self,
        scope: &MountScope,
        node: &Node,
        unique_seen: &mut HashSet<(String, String, String)>,
        unique_props: &mut HashMap<String, Vec<String>>,
    ) -> Result<bool> {
        if !unique_props.contains_key(&node.node_type) {
            let names = self.unique_property_names(scope, &node.node_type).await?;
            unique_props.insert(node.node_type.clone(), names);
        }
        let Some(names) = unique_props.get(&node.node_type) else {
            return Ok(false);
        };
        if names.is_empty() {
            return Ok(false);
        }
        let mut claims = Vec::new();
        for name in names {
            let Some(value) = node.properties.get(name) else {
                continue;
            };
            let key = (
                node.node_type.clone(),
                name.clone(),
                crate::repositories::hash_property_value(value),
            );
            if unique_seen.contains(&key) {
                return Ok(true);
            }
            claims.push(key);
        }
        unique_seen.extend(claims);
        Ok(false)
    }

    /// Property names declared `unique: true` on a node type (empty when the
    /// type is unknown — the write path makes the same assumption).
    async fn unique_property_names(
        &self,
        scope: &MountScope,
        node_type: &str,
    ) -> Result<Vec<String>> {
        use raisin_storage::{NodeTypeRepository, Storage};
        let repo = self.storage.node_types();
        let found = repo
            .get(
                raisin_storage::BranchScope::new(&scope.tenant, &scope.repo, &scope.branch),
                node_type,
                None,
            )
            .await?;
        Ok(found
            .map(|nt| crate::repositories::extract_unique_property_names(&nt))
            .unwrap_or_default())
    }
}
