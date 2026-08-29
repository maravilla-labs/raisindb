//! Staging one operation into an open transaction: resolving which node an item
//! maps onto, the etag skip-write, the foreign-node guard, and the in-chunk
//! unique-constraint guard.

use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalContext;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

/// Properties the ENGINE owns on the read path, written by `store_content`
/// rather than by a mapper. Carried across a rebuild so an upsert cannot orphan
/// a blob it never knew about; see the carry-forward in `stage_upsert`.
const CONTENT_KEYS: &[&str] = &["file", "file_size", "content_hash"];

/// The submit lifecycle, which the ENGINE owns and the mapper never reports.
///
/// A command node is both a command and a synced item, and an upsert rebuilds
/// the property map from mapper output — so the first sync after a send erased
/// the whole record of it. Observed on a paid Stripe checkout session: `status`
/// went from `sent` to absent, taking `attempt_id`, `sent_at` and
/// `sent_external_id` with it, leaving no way to tell from the node whether the
/// command had ever been issued.
///
/// It never caused a double charge — the drain claims only `queued`, and an
/// absent status is inert — but the states that most need to survive are
/// exactly the ones it destroyed. `unknown` means "this may or may not have
/// charged someone", and erasing it makes an ambiguous command indistinguishable
/// from a fresh one; `attempt_id` is the value that makes such a case
/// answerable at the provider at all. A node mid-flight at `sending` is worse
/// still: the recording CAS expects that status, so a concurrent sync could
/// wipe it and strand a command whose provider call had already happened.
///
/// Unlike [`CONTENT_KEYS`], the LIVE value wins unconditionally rather than
/// only filling a gap. On a command node these names belong to the engine, and
/// a mapper reporting a provider field called `status` is the collision this
/// exists to survive — not an adapter legitimately owning the key.
///
/// Applied ONLY to node types the mount declares in `command_node_types`.
/// `status` is an ordinary provider field elsewhere — `stripe:Subscription` and
/// `stripe:PaymentIntent` both report one — and blanket-preserving it would
/// freeze those at their first synced value.
const COMMAND_KEYS: &[&str] = &[
    "status",
    "sent_at",
    "attempt_id",
    "attempted_at",
    "sent_external_id",
    "last_error",
];

use super::index::{MountScope, SyncIndex, VirtualNodeRef};
use super::node_paths::join_path;
use super::ops::BatchOp;
use super::stamp::watched_subset;
use super::write_view::{carried_pushed_state, preserve_pending_edits, write_view_of};
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

        /// Do the mount's watched fields already hold what the provider just
        /// reported?
        ///
        /// `true` for a mount that watches nothing, which keeps a read-only
        /// mount's behaviour exactly as it was. Otherwise every watched field
        /// the incoming item carries must equal the node's current value; a
        /// field the item does not carry is not evidence of anything and is
        /// ignored rather than treated as a difference, or an item that simply
        /// omits a field would rewrite the node on every sync.
        fn watched_converged(
            existing: &super::index::VirtualNodeRef,
            incoming: &serde_json::Map<String, serde_json::Value>,
            watched_fields: &[String],
        ) -> bool {
            if watched_fields.is_empty() {
                return true;
            }
            let Some(view) = existing.write_view.as_ref() else {
                // No view means the index carried no watched values to compare,
                // so the etag is all there is. Writing is the safe answer.
                return false;
            };
            watched_fields
                .iter()
                .all(|field| match incoming.get(field) {
                    Some(value) => view.watched.get(field) == Some(value),
                    None => true,
                })
        }

        // 1. Match by __external_id within the mount subtree (survives renames).
        let (id, path) = match index.by_external(&virt.external_id) {
            Some(existing) => {
                // Etag skip-write: unchanged item → no revision churn. Bypassed
                // by a remap, which exists precisely to re-apply a mapper whose
                // output changed while the provider's item did not.
                //
                // And bypassed when the WATCHED FIELDS disagree, whatever the
                // etag says. Graph can report an isRead flip under the very
                // etag its own PATCH response returned, so trusting the etag
                // here dropped real read/unread changes for exactly the
                // messages the engine had pushed to — see `can_skip_unmapped`,
                // which no longer takes the earlier shortcut for these mounts
                // so that this comparison can happen at all.
                if !scope.force_rewrite
                    && virt.etag.is_some()
                    && existing.etag == virt.etag
                    && watched_converged(existing, &mapped.properties, &scope.watched_fields)
                {
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
                        // A path match carries no watched values to compare
                        // against (see `PathEntry`), so on a mount that watches
                        // fields the etag alone cannot justify a skip and the
                        // write is the safe answer. Read-only mounts are
                        // unaffected.
                        if virt.etag.is_some()
                            && entry.etag == virt.etag
                            && scope.watched_fields.is_empty()
                        {
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

        // LocalWins pre-merge. An upsert rebuilds the node wholesale from
        // mapper output AND reseeds `__pushed_state` from the incoming item, so
        // without this branch a pending local edit is both reverted and
        // de-nominated whenever the remote etag moved — silently, with no
        // error, which for a provider whose etag moves on the very field being
        // edited (Graph's `isRead`) means the edit is NEVER written back.
        //
        // Gated three ways so every other mount is untouched instruction for
        // instruction: the mount opted into `local_wins`, it watches fields,
        // and the item matched an EXISTING node by external id (`carried` below
        // answers exactly that) — a path-fallback or brand-new node has no
        // baseline and nothing local to preserve.
        //
        // The live node is re-read inside the open transaction rather than
        // trusting the run-start index: the index write view is loaded once and
        // refreshed only by this run's own writes, so an edit landing mid-run —
        // the exact race the stamp path's read-modify-write already defends
        // against — is invisible to it. One point read per changed-item
        // upsert, paid only by mounts that opted in. A node deleted mid-run
        // falls through to today's behaviour: the upsert recreates it from
        // remote, and the delete-reconcile walk owns what the local delete
        // meant.
        //
        // A REMAP INHERITS THE PRESERVE STEP ON EVERY MOUNT, not only the
        // `local_wins` ones. The comment below has always said a remap's
        // contract is "re-apply the mapper to unchanged remote data", not
        // "resolve conflicts in remote's favour" — but the `read_local_wins`
        // gate contradicted it, so on any other policy a `force_rewrite` both
        // reverted a pending edit to the remote value AND reseeded
        // `__pushed_state` from mapper output, which de-nominated it. The edit
        // vanished from the node, from pending, and from the next drain, in one
        // step, with nothing counted and no conflict having occurred: a remap is
        // not a remote change, so there was never anything to resolve.
        //
        // Deliberately NOT gated on `scope.force_rewrite` on the `local_wins`
        // side either: a remap's contract is the same there.
        //
        // Known edge, documented rather than solved: `name` is not a property
        // and not pushable via `state_only`, so a name derived from a pending
        // watched field still follows the remote item.
        //
        // An incoming DELETE is also not preserved (see the Delete arm above):
        // a `state_only` edit is a patch against a remote object, and when the
        // provider says the object is gone there is nothing left to patch —
        // `local_wins` never promised resurrection, and the 409 force path
        // cannot recreate either (it re-sends an update, not a create).
        let carried = index
            .by_external(&virt.external_id)
            .and_then(|existing| existing.pushed_state().cloned());
        let matched_existing = index.by_external(&virt.external_id).is_some();
        let wants_preserve = (scope.read_local_wins || scope.force_rewrite)
            && !scope.watched_fields.is_empty()
            && matched_existing;
        // Content properties are written by `store_content` AFTER the upsert
        // that created the node, so the mapper never emits them and a wholesale
        // rebuild erases them — see the carry-forward below.
        let wants_content_carry = matched_existing
            && CONTENT_KEYS
                .iter()
                .any(|k| !mapped.properties.contains_key(*k));

        // At most ONE live read, shared by both. Both need the node as it stands
        // inside this transaction rather than the run-start index, which is
        // refreshed only by this run's own writes and so cannot see an edit that
        // landed mid-run.
        let live = if wants_preserve || wants_content_carry {
            tx.get_node(&scope.workspace, &id).await?
        } else {
            None
        };

        let preserved = if wants_preserve {
            live.as_ref().and_then(|node| {
                write_view_of(node, &scope.watched_fields).and_then(|view| {
                    preserve_pending_edits(&mapped.properties, &view, &scope.watched_fields)
                })
            })
        } else {
            None
        };

        // The item the provider just reported IS the pushed state: seeding
        // `__pushed_state` from the mapper's own output is what makes a REMOTE
        // change converge on arrival instead of looking like a local edit and
        // being pushed straight back. (Under `local_wins`, `preserved` above
        // has already carved the pending fields out of both maps; everything
        // else still converges exactly this way.)
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
        //
        // (The `.or(carried)` fallback and the preserve branch can never both
        // fire: `preserved` requires watched fields, under which
        // `watched_subset` always answers `Some`.)
        let (merged_props, pushed_state) = match preserved {
            Some((merged, baseline)) => (Some(merged), Some(baseline)),
            None => (
                None,
                watched_subset(&mapped.properties, &scope.watched_fields).or(carried),
            ),
        };
        let base_props = merged_props.as_ref().unwrap_or(&mapped.properties);

        // CONTENT PROPERTIES SURVIVE A REBUILD.
        //
        // `file`, `file_size` and `content_hash` are written by `store_content`
        // after the node exists — the mapper cannot know a blob id it has never
        // seen — so an upsert that rebuilds the property map from mapper output
        // erases them. The trigger is ordinary: an attachment child inherits its
        // parent's etag, so any change to the message re-stages the child and
        // orphaned its blob, leaving a node that claims a file and points at
        // nothing.
        //
        // Carried forward from the live node only when the mapper did not supply
        // them itself, so an adapter that genuinely owns these keys still wins.
        // The carry is keyed on the child's own change token by construction: a
        // child whose provider etag really moved is re-fetched by `store_content`
        // anyway, and one that did not never needed to lose them.
        let carried_content: Option<Map<String, Value>> = live.as_ref().and_then(|node| {
            let mut out: Option<Map<String, Value>> = None;
            for key in CONTENT_KEYS {
                if base_props.contains_key(*key) {
                    continue;
                }
                if let Some(value) = node
                    .properties
                    .get(*key)
                    .and_then(|pv| serde_json::to_value(pv).ok())
                {
                    out.get_or_insert_with(|| base_props.clone())
                        .insert((*key).to_string(), value);
                }
            }
            out
        });
        let base_props = carried_content.as_ref().unwrap_or(base_props);

        // The submit lifecycle survives a rebuild — see `COMMAND_KEYS`.
        let is_command = scope
            .command_node_types
            .iter()
            .any(|t| t == &mapped.node_type);
        let carried_command: Option<Map<String, Value>> = if is_command {
            live.as_ref().and_then(|node| {
                let mut out: Option<Map<String, Value>> = None;
                for key in COMMAND_KEYS {
                    if let Some(value) = node
                        .properties
                        .get(*key)
                        .and_then(|pv| serde_json::to_value(pv).ok())
                    {
                        out.get_or_insert_with(|| base_props.clone())
                            .insert((*key).to_string(), value);
                    }
                }
                out
            })
        } else {
            None
        };
        let effective_props = carried_command.as_ref().unwrap_or(base_props);

        let node = Node {
            id,
            node_type: mapped.node_type.clone(),
            name,
            path: path.clone(),
            workspace: Some(scope.workspace.clone()),
            properties: build_properties(effective_props, virt, pushed_state.as_ref()),
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
            // Already computed above for the lifecycle carry.
            is_command,
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
