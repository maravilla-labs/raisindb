//! EventHandler trait implementation for the unified job event handler
//!
//! Routes incoming events to the appropriate handler methods based on event type.

use super::UnifiedJobEventHandler;
use raisin_events::{
    Event, EventHandler, NodeEventKind, ReplicationEventKind, RepositoryEventKind,
};
use std::future::Future;
use std::pin::Pin;

impl EventHandler for UnifiedJobEventHandler {
    fn name(&self) -> &str {
        "unified_job_handler"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                Event::Node(node_event) => {
                    let result = match &node_event.kind {
                        NodeEventKind::Created | NodeEventKind::Updated => {
                            self.handle_node_change(node_event).await
                        }
                        NodeEventKind::Deleted => self.handle_node_delete(node_event).await,
                        _ => return Ok(()),
                    };

                    // Best-effort: log errors but don't fail the event
                    if let Err(e) = &result {
                        tracing::error!(
                            error = %e,
                            node_id = %node_event.node_id,
                            event_kind = ?node_event.kind,
                            "Error processing node event"
                        );
                    }

                    Ok(())
                }
                Event::Repository(repo_event) => {
                    if repo_event.kind == RepositoryEventKind::BranchCreated {
                        let result = self.handle_branch_created(repo_event).await;

                        if let Err(e) = &result {
                            tracing::error!(
                                error = %e,
                                repo_id = %repo_event.repository_id,
                                branch = ?repo_event.branch_name,
                                "Error processing branch creation event"
                            );
                        }
                    }

                    Ok(())
                }
                Event::Replication(repl_event) => {
                    if repl_event.kind == ReplicationEventKind::OperationBatchApplied {
                        let result = self.handle_operation_batch_applied(repl_event).await;

                        if let Err(e) = &result {
                            tracing::error!(
                                error = %e,
                                tenant_id = %repl_event.tenant_id,
                                repo_id = %repl_event.repository_id,
                                operation_count = repl_event.operation_count,
                                "Error processing replication event"
                            );
                        }
                    }

                    Ok(())
                }
                Event::Schema(schema_event) => {
                    let result = self.handle_schema_change(schema_event).await;

                    if let Err(e) = &result {
                        tracing::error!(
                            error = %e,
                            schema_id = %schema_event.schema_id,
                            schema_type = %schema_event.schema_type,
                            event_kind = ?schema_event.kind,
                            "Error processing schema event"
                        );
                    }

                    Ok(())
                }
                _ => Ok(()),
            }
        })
    }
}
