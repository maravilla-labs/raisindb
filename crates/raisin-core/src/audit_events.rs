//! Event-bus audit subscriber.
//!
//! Writes audit-log entries for committed node changes by subscribing to the
//! event bus. Because **every** write path — the node API (`NodeService`), SQL
//! DML, and QuickJS/Starlark functions — commits through the same transaction
//! layer, which emits a `NodeEvent` per changed node, this single subscriber
//! audits them all uniformly. Gated on the NodeType `auditable` flag.
//!
//! This replaces the older per-call `NodeService` audit hook (which only covered
//! the node-API path). The actor is read from `event.metadata.actor`, stamped at
//! commit time (see `raisin-rocksdb` `emit_node_events`).

use std::pin::Pin;
use std::sync::Arc;

use raisin_audit::AuditRepository;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{
    BranchScope, Event, EventHandler, NodeEventKind, NodeTypeRepository, Storage,
};

/// Subscribes to node lifecycle events and persists audit-log entries for
/// NodeTypes marked `auditable`.
pub struct AuditEventHandler<S, A>
where
    S: Storage,
    A: AuditRepository,
{
    audit: Arc<A>,
    storage: Arc<S>,
}

impl<S, A> AuditEventHandler<S, A>
where
    S: Storage,
    A: AuditRepository,
{
    pub fn new(audit: Arc<A>, storage: Arc<S>) -> Self {
        Self { audit, storage }
    }

    /// Whether the given NodeType opts into audit logging. Resolution failures
    /// are treated as "not auditable" so a transient lookup error never blocks
    /// the write that already committed.
    async fn is_auditable(&self, tenant: &str, repo: &str, branch: &str, node_type: &str) -> bool {
        self.storage
            .node_types()
            .get(BranchScope::new(tenant, repo, branch), node_type, None)
            .await
            .ok()
            .flatten()
            .map(|nt| nt.auditable())
            .unwrap_or(false)
    }
}

impl<S, A> EventHandler for AuditEventHandler<S, A>
where
    S: Storage + Send + Sync + 'static,
    A: AuditRepository + 'static,
{
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let Event::Node(e) = event else {
                return Ok(());
            };

            // Map the committed change to an audit action. Only content
            // lifecycle changes are audited; ordering/relation events are not.
            let action = match &e.kind {
                NodeEventKind::Created => AuditLogAction::Create,
                NodeEventKind::Updated => AuditLogAction::Update,
                NodeEventKind::Deleted => AuditLogAction::Delete,
                NodeEventKind::Published => AuditLogAction::Publish,
                NodeEventKind::Unpublished => AuditLogAction::Unpublish,
                NodeEventKind::PropertyChanged { .. } => AuditLogAction::UpdateProperty,
                NodeEventKind::Reordered
                | NodeEventKind::RelationAdded { .. }
                | NodeEventKind::RelationRemoved { .. } => return Ok(()),
            };

            // Gate on the NodeType `auditable` flag.
            let Some(node_type) = e.node_type.as_deref() else {
                return Ok(());
            };
            if !self
                .is_auditable(&e.tenant_id, &e.repository_id, &e.branch, node_type)
                .await
            {
                return Ok(());
            }

            // Actor is stamped onto every event at commit time.
            let user_id = e
                .metadata
                .as_ref()
                .and_then(|m| m.get("actor"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let log = raisin_audit::make_log(
                e.node_id.clone(),
                e.path.clone().unwrap_or_default(),
                e.workspace_id.clone(),
                user_id,
                action,
                None,
            );
            self.audit.write_log(log).await?;
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "audit"
    }
}
