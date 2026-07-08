//! Reference retarget job handler
//!
//! When a node MOVEs, the reference-index REVERSE entries (keyed by the target's
//! stable id) survive, but the FORWARD entries of nodes that reference it embed
//! a denormalized snapshot `{workspace, id, path}` whose `path` goes stale. The
//! direct (editor/API) move path can't do this inline, so a move emits an
//! `Updated` event tagged `moved:true`, which enqueues one `RetargetReferences`
//! job per moved node. This handler rewrites every referrer's forward `path` to
//! the moved node's new path.

use raisin_error::{Error, Result};
use raisin_events::EventBus;
use raisin_models::nodes::properties::RaisinReference;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::scope::{RepoScope, StorageScope};
use raisin_storage::{ReferenceIndexRepository, WorkspaceRepository};
use rocksdb::DB;
use std::sync::Arc;

use crate::repositories::{ReferenceIndexRepositoryImpl, WorkspaceRepositoryImpl};

/// Handler for `RetargetReferences` jobs.
pub struct RetargetReferencesHandler {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
}

impl RetargetReferencesHandler {
    pub fn new(db: Arc<DB>, event_bus: Arc<dyn EventBus>) -> Self {
        Self { db, event_bus }
    }

    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        let (node_id, target_workspace, new_path) = match &job.job_type {
            JobType::RetargetReferences {
                node_id,
                workspace,
                new_path,
            } => (node_id.as_str(), workspace.as_str(), new_path.as_str()),
            _ => {
                return Err(Error::Validation(
                    "Expected RetargetReferences job type".to_string(),
                ))
            }
        };

        let tenant_id = &context.tenant_id;
        let repo_id = &context.repo_id;
        let branch = &context.branch;

        let ref_index = ReferenceIndexRepositoryImpl::new(self.db.clone());
        let ws_repo = WorkspaceRepositoryImpl::new(self.db.clone(), self.event_bus.clone());

        // Referrers can live in any workspace; the reverse index is partitioned
        // by the referrer's workspace, so scan each workspace in the repo.
        let workspaces = ws_repo.list(RepoScope::new(tenant_id, repo_id)).await?;

        // The moved node is fully identified by its stable id + workspace; the
        // reference envelope only needs {id, workspace, path}.
        let new_reference = RaisinReference {
            id: node_id.to_string(),
            workspace: target_workspace.to_string(),
            path: new_path.to_string(),
        };

        let mut retargeted = 0usize;
        for ws in &workspaces {
            let scope = StorageScope::new(tenant_id, repo_id, branch, &ws.name);
            for published in [false, true] {
                let referrers = ref_index
                    .find_referencing_nodes(scope, target_workspace, node_id, published)
                    .await?;
                for (referrer_id, property_path) in referrers {
                    ref_index
                        .retarget_forward_path(
                            scope,
                            &referrer_id,
                            &property_path,
                            &new_reference,
                            &context.revision,
                            published,
                        )
                        .await?;
                    retargeted += 1;
                }
            }
        }

        tracing::info!(
            job_id = %job.id,
            node_id = %node_id,
            workspace = %target_workspace,
            new_path = %new_path,
            workspaces_scanned = workspaces.len(),
            retargeted,
            "Retargeted reference forward paths after move"
        );

        Ok(())
    }
}
