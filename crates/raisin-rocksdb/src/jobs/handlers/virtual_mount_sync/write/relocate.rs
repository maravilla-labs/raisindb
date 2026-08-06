//! Moves ACROSS the mount boundary: a node dragged out of the mount, and a node
//! dragged into it.
//!
//! # Why this is not `move_policy`
//!
//! `move_policy` (`write::policy`) is about a move WITHIN the mount — a changed
//! value of a location field the adapter declared, pushed as an ordinary
//! `update`. It has a remote counterpart by construction: the provider has
//! folders, and one of them is the destination.
//!
//! Crossing the boundary has none. There is no provider folder for "outside
//! this mount", so `push` is not expressible and `reject` cannot un-move
//! anything — nothing in the engine reverses a user's move, and the read path
//! deliberately preserves an existing node's path on upsert. The only honest
//! answers are the two implemented here, and neither is configurable:
//!
//! * **Out → detach.** The node keeps its content and loses its provenance
//!   (`__mount_id`, `__external_id`, `__etag`, `__pushed_state`, `__synced_at`).
//!   It becomes ordinary local content and the remote is untouched.
//! * **In → reject.** Nothing is adopted, nothing is pushed, and the operator is
//!   told. Local CREATE propagation does not exist (see [`super`]), so there is
//!   no reading of `mirror` + `can_create` under which a foreign node currently
//!   becomes a provider object.
//!
//! # Why the reconcile walk and not the drain
//!
//! The drain works from `SyncIndex`, and the index is a path-prefix scan of the
//! mount path (`materializer::load_index_nodes`). A node that has LEFT the mount
//! path is therefore not in the index at all: not as a candidate, not as a
//! deletion, not as anything. It is invisible to every part of the drain, which
//! is exactly why the divergence it causes was silent — the node keeps its
//! `__external_id` forever, and the next full reconcile, finding no local node
//! for that provider item, re-imports it as a DUPLICATE.
//!
//! The revision walk sees it because the walk is keyed by commit, not by path.

use std::sync::Arc;

use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage};

use super::super::config::{MountConfig, SYNC_ACTOR};
use super::super::materializer::{node_external_id, node_mount_id, under, PUSHED_STATE_PROP};
use crate::RocksDBStorage;

/// Reserved properties a detach strips. Everything else about the node — its
/// type, name, path, and every mapped property — is left exactly as it is: the
/// user moved it because they want to keep it.
const PROVENANCE_PROPS: [&str; 4] = ["__mount_id", "__external_id", "__etag", "__synced_at"];

/// What one changed node turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Relocation {
    /// A mount-owned node now sits outside the mount path; it was detached.
    Out,
    /// A node that is nobody's virtual node now sits inside the mount path.
    In,
}

/// [`handle_change`] for one reported change, swallowing its errors.
///
/// One unreadable node must not stop the walk: the deletes in the same revision
/// are the half that cannot be recovered later, and abandoning the pass to
/// re-run it would only keep hitting the same node.
pub(crate) async fn probe(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    mount: &MountConfig,
    change: &raisin_storage::NodeChangeInfo,
) -> Option<Relocation> {
    match handle_change(
        storage,
        tenant,
        repo,
        mount,
        &change.workspace,
        &change.node_id,
    )
    .await
    {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(
                mount_id = %mount.mount_id,
                node_id = %change.node_id,
                error = %e,
                "mount-boundary probe failed for a changed node; continuing"
            );
            None
        }
    }
}

/// Classify one changed node and, if it left the mount, detach it.
///
/// Returns `None` for the overwhelmingly common case — a node that is where it
/// has always been — which costs one point read and no write.
async fn handle_change(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    mount: &MountConfig,
    workspace: &str,
    node_id: &str,
) -> Result<Option<Relocation>> {
    let scope = raisin_storage::BranchScope::new(tenant, repo, &mount.target_branch)
        .with_workspace(workspace);
    // Read at HEAD, not at the revision that reported the change. The question
    // is "where is this node NOW", and answering it at the reporting revision
    // would detach a node that has since been moved back — and then, one pass
    // later, re-import it as a duplicate. Reading HEAD also makes the walk
    // idempotent under a re-read of the same revision.
    let Some(node) = storage.nodes().get(scope, node_id, None).await? else {
        // Deleted since. The delete path owns it (and has its own pre-image
        // recovery); there is nothing to relocate.
        return Ok(None);
    };

    let inside = under(&mount.mount_path, &node.path);
    match node_mount_id(&node) {
        Some(owner) if owner == mount.mount_id => {
            if inside {
                // A move WITHIN the mount. Nothing to do here: the local path
                // sticks (the read path preserves it on upsert) and any provider
                // consequence is a location FIELD, which the drain pushes under
                // `move_policy`. Detaching here would punish an ordinary
                // rearrangement of a mounted folder tree.
                return Ok(None);
            }
            detach(storage, tenant, repo, mount, workspace, node).await?;
            Ok(Some(Relocation::Out))
        }
        // Another mount's node. Not ours to detach and not ours to adopt; the
        // other mount's own walk sees it as a move out of ITS path if it is one.
        Some(_) => Ok(None),
        None if inside && node_external_id(&node).is_none() => {
            tracing::warn!(
                mount_id = %mount.mount_id,
                node_id = %node.id,
                path = %node.path,
                mount_path = %mount.mount_path,
                "a node that this mount does not own now sits under its path. It is NOT \
                 adopted and nothing is created at the provider — local create propagation \
                 does not exist. It will stay local, and a sync that needs this path for an \
                 incoming item will skip that item rather than overwrite this node."
            );
            Ok(Some(Relocation::In))
        }
        None => Ok(None),
    }
}

/// Strip a node's virtual provenance, as the sync actor.
///
/// Committed as [`SYNC_ACTOR`] so the next walk skips this revision as an echo
/// rather than re-examining a node it has already finished with.
///
/// The consequence is stated plainly because it is inherent and will otherwise
/// be reported as a bug: the provider still has the object, this mount no longer
/// has a node claiming it, so the next reconcile RE-IMPORTS it at its template
/// path. The user keeps the copy they moved; a fresh mounted copy appears. That
/// is the same bargain `delete_policy: detach` makes, for the same reason —
/// there is no per-mount suppression set anywhere in the engine.
async fn detach(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    mount: &MountConfig,
    workspace: &str,
    node: Node,
) -> Result<()> {
    let external_id = node_external_id(&node).unwrap_or("").to_string();
    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(&mount.target_branch)?;
    tx.set_actor(SYNC_ACTOR)?;
    tx.set_auth_context(AuthContext::system_as(SYNC_ACTOR))?;
    tx.set_message("virtual mount: detach a node moved out of the mount")?;

    // Re-read inside the transaction rather than writing back the node this
    // function was handed: the read above is unsynchronized, and writing a
    // whole node from it would silently revert any property a user changed in
    // between. Only the reserved keys are removed.
    let Some(mut current) = tx.get_node(workspace, &node.id).await? else {
        return Ok(());
    };
    for key in PROVENANCE_PROPS {
        current.properties.remove(key);
    }
    current.properties.remove(PUSHED_STATE_PROP);
    tx.upsert_node(workspace, &current).await?;
    tx.commit().await?;

    tracing::info!(
        mount_id = %mount.mount_id,
        node_id = %node.id,
        external_id = %external_id,
        path = %node.path,
        mount_path = %mount.mount_path,
        "a mount-owned node was moved out of the mount path; detaching it. The node keeps \
         its content and becomes ordinary local content, the remote object is untouched, \
         and a later reconcile will re-import that object as a NEW node under the mount."
    );
    Ok(())
}
