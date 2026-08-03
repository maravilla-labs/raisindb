//! Event handler for invalidating the SQL workspace-catalog cache.
//!
//! The catalog (default nodes schema + one table per workspace) is derived once
//! per repo and shared by every transport. This handler is what makes that
//! caching correct rather than merely fast: it listens for `Event::Workspace`
//! and drops the affected repo's entry, so a workspace created a moment ago is
//! queryable immediately instead of after the cache's TTL floor expires.
//!
//! Deliberately mirrors `SchemaStatsEventHandler`, which does the same job for
//! the planner's schema statistics.

use raisin_events::{Event, EventHandler};
use std::future::Future;
use std::pin::Pin;

/// Invalidates cached SQL catalogs when workspaces are created, updated or deleted.
pub struct WorkspaceCatalogEventHandler;

impl EventHandler for WorkspaceCatalogEventHandler {
    fn name(&self) -> &str {
        "workspace_catalog_invalidator"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Workspace(workspace_event) = event {
                raisin_sql_execution::invalidate_workspace_catalog(
                    &workspace_event.tenant_id,
                    &workspace_event.repository_id,
                );
                tracing::debug!(
                    tenant = %workspace_event.tenant_id,
                    repo = %workspace_event.repository_id,
                    workspace = %workspace_event.workspace,
                    kind = ?workspace_event.kind,
                    "Workspace catalog cache invalidated"
                );
            }
            Ok(())
        })
    }
}
