//! Execution of the `SPATIAL INDEX` admin statements.
//!
//! # What each statement touches
//!
//! - `SHOW … CONFIG` reads the **replicated intent** (`WorkspaceConfig.spatial`).
//! - `SHOW … HEALTH` / `VERIFY` read **local reality**: the per-node index state
//!   record plus a physical census of the keys that are actually present. Reporting
//!   only the state record would be reporting intent twice.
//! - `ALTER` / `RESET` write the workspace record, which is replicated — that IS
//!   the cluster-wide fan-out mechanism, because every peer then observes its own
//!   policy-hash drift.
//! - `REBUILD` queues a **local** job. It is deliberately not broadcast; the result
//!   row says so, because an operator assuming a cluster-wide repair would leave
//!   peers stale.

mod report;

use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::{
    sorted_precisions, PropertyValue, SpatialCoverMode, SpatialPropertySchema,
    SpatialWorkspaceSchema,
};
use raisin_sql::ast::spatial_admin::{CoverModeSpec, SpatialAdminStatement, SpatialIndexSettings};
use raisin_storage::scope::RepoScope;
use raisin_storage::spatial_admin::SpatialIndexAdmin;
use raisin_storage::{Storage, WorkspaceRepository};
use std::sync::Arc;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Authorize a spatial admin statement.
    ///
    /// The ladder is copied verbatim from `acl_check_authorization`: same order, same
    /// error shapes. A parallel mechanism here would be a second thing to get wrong,
    /// and this one has already been reviewed in place.
    fn spatial_admin_check_authorization(&self, stmt: &SpatialAdminStatement) -> Result<(), Error> {
        let auth = self.auth_context.as_ref().ok_or_else(|| {
            Error::Forbidden("Authentication required for spatial index administration".to_string())
        })?;

        // System context always allowed (internal operations).
        if auth.is_system {
            return Ok(());
        }

        if auth.is_anonymous {
            return Err(Error::Forbidden(
                "Anonymous users cannot administer the spatial index".to_string(),
            ));
        }

        // SHOW is read-only and needs authentication only. VERIFY is NOT in that
        // set: it performs an unbounded scan of the index.
        if stmt.is_read_only() {
            return Ok(());
        }

        if let Some(ref perms) = auth.resolved_permissions {
            if perms.is_system_admin {
                return Ok(());
            }
        }

        if auth.has_role("system_admin") {
            return Ok(());
        }

        Err(Error::Forbidden(format!(
            "Insufficient permissions: {} requires the system_admin role",
            stmt.operation()
        )))
    }

    /// Execute a spatial admin statement.
    pub(crate) async fn execute_spatial_admin(
        &self,
        stmt: &SpatialAdminStatement,
    ) -> Result<RowStream, Error> {
        tracing::info!("Executing spatial admin statement: {}", stmt);

        // Authorization BEFORE any mutation, matching `execute_acl`'s ordering.
        self.spatial_admin_check_authorization(stmt)?;

        match stmt {
            SpatialAdminStatement::ShowConfig {
                workspace,
                property,
            } => {
                self.spatial_show_config(workspace.as_deref(), property.as_deref())
                    .await
            }
            SpatialAdminStatement::ShowHealth {
                workspace,
                property,
            } => {
                self.spatial_show_health(workspace.as_deref(), property.as_deref())
                    .await
            }
            SpatialAdminStatement::Alter {
                workspace,
                property,
                settings,
            } => {
                self.spatial_alter(workspace, property.as_deref(), settings)
                    .await
            }
            SpatialAdminStatement::Reset {
                workspace,
                property,
            } => self.spatial_reset(workspace, property.as_deref()).await,
            SpatialAdminStatement::Rebuild {
                workspace,
                property,
            } => self.spatial_rebuild(workspace, property.as_deref()).await,
            SpatialAdminStatement::Verify {
                workspace,
                property,
            } => self.spatial_verify(workspace, property.as_deref()).await,
        }
    }

    /// The backend's spatial admin handle, or a clear error.
    ///
    /// `None` means the backend has no spatial index to administer — reported rather
    /// than silently succeeding, which is the failure mode the `BulkSql` debug stub
    /// taught this codebase to avoid.
    pub(super) fn spatial_admin_handle(&self) -> Result<Arc<dyn SpatialIndexAdmin>, Error> {
        self.storage.spatial_admin().ok_or_else(|| {
            Error::Validation(
                "This storage backend does not support spatial index administration".to_string(),
            )
        })
    }

    /// Write the declared policy into the workspace record.
    async fn spatial_alter(
        &self,
        workspace: &str,
        property: Option<&str>,
        settings: &SpatialIndexSettings,
    ) -> Result<RowStream, Error> {
        let scope = RepoScope::new(&self.tenant_id, &self.repo_id);
        let mut ws = self
            .storage
            .workspaces()
            .get(scope, workspace)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Workspace '{}' not found", workspace)))?;

        // Merge onto whatever this scope already declared, so setting one field does
        // not silently clear the others.
        let existing_scope: SpatialPropertySchema = ws
            .config
            .spatial
            .as_ref()
            .map(|s| s.scope(property))
            .unwrap_or_default();

        let merged = merge_settings(existing_scope, settings);

        // `SpatialWorkspaceSchema::edited` is the ONE place a scoped edit is
        // expressed, so this and the storage layer cannot drift on what a scope means.
        ws.config.spatial = SpatialWorkspaceSchema::edited(
            ws.config.spatial.clone(),
            property,
            Some(merged.clone()),
        );

        self.storage.workspaces().put(scope, ws).await?;

        let mut row = Row::new();
        row.insert(
            "workspace".to_string(),
            PropertyValue::String(workspace.to_string()),
        );
        row.insert(
            "property".to_string(),
            PropertyValue::String(property.unwrap_or("*").to_string()),
        );
        row.insert(
            "precisions".to_string(),
            PropertyValue::String(report::format_precisions(
                merged.precisions.as_deref().unwrap_or(&[]),
            )),
        );
        row.insert(
            "cover".to_string(),
            PropertyValue::String(format!(
                "{:?}",
                merged.cover.unwrap_or(SpatialCoverMode::Centroid)
            )),
        );
        // Said out loud because it is the single most misunderstood part of the
        // design: the config change replicates, the index rebuild does not.
        row.insert(
            "note".to_string(),
            PropertyValue::String(
                "configuration written and replicated; run REBUILD SPATIAL INDEX on each node \
                 to migrate existing entries"
                    .to_string(),
            ),
        );
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    /// Drop the declared policy for this scope.
    async fn spatial_reset(
        &self,
        workspace: &str,
        property: Option<&str>,
    ) -> Result<RowStream, Error> {
        let scope = RepoScope::new(&self.tenant_id, &self.repo_id);
        let mut ws = self
            .storage
            .workspaces()
            .get(scope, workspace)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Workspace '{}' not found", workspace)))?;

        ws.config.spatial =
            SpatialWorkspaceSchema::edited(ws.config.spatial.clone(), property, None);
        self.storage.workspaces().put(scope, ws).await?;

        let mut row = Row::new();
        row.insert(
            "workspace".to_string(),
            PropertyValue::String(workspace.to_string()),
        );
        row.insert(
            "property".to_string(),
            PropertyValue::String(property.unwrap_or("*").to_string()),
        );
        row.insert(
            "result".to_string(),
            PropertyValue::String("configuration reset to inherited defaults".to_string()),
        );
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    /// Queue a LOCAL rebuild.
    async fn spatial_rebuild(
        &self,
        workspace: &str,
        property: Option<&str>,
    ) -> Result<RowStream, Error> {
        let admin = self.spatial_admin_handle()?;
        let branch = self.effective_branch().await;

        let job_id = admin
            .queue_build(
                &self.tenant_id,
                &self.repo_id,
                &branch,
                workspace,
                property,
                // A rebuild tombstones whatever is physically present before
                // re-emitting, which is what lets a SHRINKING precision set actually
                // remove the entries it no longer wants. A gap-fill (`false`) cannot.
                true,
            )
            .await?;

        let mut row = Row::new();
        row.insert("job_id".to_string(), PropertyValue::String(job_id));
        row.insert(
            "workspace".to_string(),
            PropertyValue::String(workspace.to_string()),
        );
        row.insert(
            "property".to_string(),
            PropertyValue::String(property.unwrap_or("*").to_string()),
        );
        row.insert(
            "scope".to_string(),
            PropertyValue::String("local node only".to_string()),
        );
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }
}

/// Fold parsed settings onto whatever the scope already declared.
fn merge_settings(
    mut existing: SpatialPropertySchema,
    settings: &SpatialIndexSettings,
) -> SpatialPropertySchema {
    if let Some(precisions) = &settings.precisions {
        // Canonicalised here as well as in the model layer so the value that goes
        // into the replicated record is already the one `policy_hash` will see.
        existing.precisions = Some(sorted_precisions(precisions.clone()));
    }
    if let Some(srid) = settings.srid {
        existing.srid = Some(srid);
    }
    if let Some(bucket) = &settings.bucket_property {
        existing.bucket_property = Some(bucket.clone());
    }
    if let Some(cover) = settings.cover {
        existing.cover = Some(match cover {
            CoverModeSpec::Centroid => SpatialCoverMode::Centroid,
            CoverModeSpec::Extent => SpatialCoverMode::Extent,
        });
    }
    existing
}

#[cfg(test)]
mod tests;
