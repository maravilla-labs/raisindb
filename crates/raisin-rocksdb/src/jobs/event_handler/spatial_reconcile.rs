//! Automatic spatial index reconciliation.
//!
//! The spatial index itself is written **inline in the write batch**, on every path
//! including replication apply — so unlike fulltext there is no per-node indexing job
//! to enqueue here. What this arm does is notice that the *configured* policy no
//! longer matches what the local index was *built* under, and schedule a local
//! rebuild.
//!
//! It runs for LOCAL AND REMOTE events, for the same reason the fulltext arm does:
//! every instance maintains its own local index from replicated records, so every
//! instance must reconcile independently. That is also how a configuration change
//! fans out across a masterless cluster with nothing to broadcast —
//! `WorkspaceConfig.spatial` is replicated data, so each peer sees the new policy
//! arrive and each peer reconciles itself.

use super::UnifiedJobEventHandler;
use crate::spatial_state::{BuildTrigger, SpatialStateStore};
use raisin_events::NodeEvent;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobContext, JobType};

impl UnifiedJobEventHandler {
    /// Schedule a local spatial build when configured intent and local reality
    /// have drifted apart.
    ///
    /// Three triggers, all resolved by
    /// [`crate::spatial_state::resolve_write_policy`] so this arm and the write
    /// path cannot disagree about what "drifted" means:
    ///
    /// * **no state record** — pre-existing data, or a property this instance has
    ///   never indexed. Gap-fill (`rebuild: false`).
    /// * **phase `NotBuilt`** — known-missing entries. Gap-fill.
    /// * **`policy_hash` mismatch** — the precision set / cover / bucket changed.
    ///   Full rebuild (`rebuild: true`), because a shrinking precision set has to
    ///   REMOVE the entries it no longer wants, which a gap-fill never does.
    ///
    /// The mismatch trigger is the one that used to be missing: only `NotBuilt`
    /// and a missing record enqueued anything, so an already-built index was
    /// never rebuilt after a policy change and the change took effect on nothing.
    pub(crate) async fn reconcile_spatial_index(
        &self,
        node_event: &NodeEvent,
        context: &JobContext,
    ) {
        // Cheap gate: only nodes that actually carry a geometry are interesting, and
        // the event metadata usually already holds the node so this costs no read.
        let node: Option<raisin_models::nodes::Node> = node_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("node_data"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let Some(node) = node else {
            // Without the node in the event we would have to read it back just to
            // find out whether it has geometry. Skip: the integrity sweep and the
            // explicit admin rebuild endpoint both still cover this case, and the
            // index entries themselves were already written inline by the write
            // that produced this event.
            return;
        };

        let geometry_properties: Vec<&String> = node
            .properties
            .iter()
            .filter(|(_, v)| matches!(v, PropertyValue::Geometry(_)))
            .map(|(k, _)| k)
            .collect();

        if geometry_properties.is_empty() {
            return;
        }

        let state_store = SpatialStateStore::new(self.storage_db());

        for property in geometry_properties {
            let resolution = state_store.write_policy(
                &node_event.tenant_id,
                &node_event.repository_id,
                &node_event.branch,
                &node_event.workspace_id,
                property,
            );

            let Some(trigger) = resolution.needs_build else {
                continue;
            };

            if let BuildTrigger::PolicyChanged {
                indexed_hash,
                configured_hash,
            } = trigger
            {
                tracing::info!(
                    tenant = %node_event.tenant_id,
                    repo = %node_event.repository_id,
                    branch = %node_event.branch,
                    workspace = %node_event.workspace_id,
                    property = %property,
                    indexed_hash,
                    configured_hash,
                    indexed_precisions = ?resolution.indexed.as_ref().map(|p| &p.precisions),
                    configured_precisions = ?resolution.configured.precisions,
                    "Spatial policy changed; queueing a local rebuild"
                );
            }

            // Idempotent: the dedup key is
            // `spatial:{tenant}:{repo}:{branch}:{ws}:{property}` and only matches
            // ACTIVE jobs, so a burst of writes under a changed policy collapses
            // onto one job while it runs, and a later change enqueues again.
            if let Err(e) = self
                .enqueue_job(
                    JobType::SpatialIndexBuild {
                        tenant_id: node_event.tenant_id.clone(),
                        repo_id: node_event.repository_id.clone(),
                        branch: node_event.branch.clone(),
                        workspace: node_event.workspace_id.clone(),
                        property: Some(property.clone()),
                        rebuild: trigger.rebuild(),
                    },
                    context,
                )
                .await
            {
                tracing::warn!(
                    error = %e,
                    property = %property,
                    workspace = %node_event.workspace_id,
                    "Failed to enqueue spatial index build job"
                );
            }
        }
    }
}
