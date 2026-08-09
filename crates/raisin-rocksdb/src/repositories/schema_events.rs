//! Local `Event::Schema` emission for the schema repositories.
//!
//! `Event::Schema` used to be published from exactly one place — the replication
//! applicator (`replication/application/applicator/mod.rs::emit_schema_event`,
//! `source = "replication"`). The node that ORIGINATES a schema edit therefore
//! published nothing, so every consumer (the schema-stats cache invalidator, WS
//! subscribers) only ever learned about schema changes that arrived from a peer.
//! On a single-node deployment that means never.
//!
//! The fix emits from the repository layer, which is the single choke point that
//! all 20+ local write call sites (HTTP handlers, system updates, SQL DDL,
//! nodetype init, …) funnel through. Emitting per call site would mean 20+
//! mirrored copies — the drift that CLAUDE.md names as this codebase's #1
//! recurring bug class.
//!
//! There is no double-fire risk: the applicator writes directly to the column
//! families precisely to avoid re-entering these repositories (see its module
//! doc, "Direct CF Writes"), so a replicated change never reaches this code.

use raisin_events::{Event, EventBus, SchemaEvent, SchemaEventKind};
use raisin_storage::scope::BranchScope;
use std::collections::HashMap;
use std::sync::Arc;

/// Which schema surface a local write touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaKindFamily {
    NodeType,
    Archetype,
    ElementType,
}

/// What happened to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaChange {
    Created,
    Updated,
    Deleted,
}

impl SchemaKindFamily {
    /// The `schema_type` string carried on the event, matching the values the
    /// replication applicator emits.
    fn schema_type(self) -> &'static str {
        match self {
            SchemaKindFamily::NodeType => "NodeType",
            SchemaKindFamily::Archetype => "Archetype",
            SchemaKindFamily::ElementType => "ElementType",
        }
    }

    fn event_kind(self, change: SchemaChange) -> SchemaEventKind {
        match (self, change) {
            (SchemaKindFamily::NodeType, SchemaChange::Created) => SchemaEventKind::NodeTypeCreated,
            (SchemaKindFamily::NodeType, SchemaChange::Updated) => SchemaEventKind::NodeTypeUpdated,
            (SchemaKindFamily::NodeType, SchemaChange::Deleted) => SchemaEventKind::NodeTypeDeleted,
            (SchemaKindFamily::Archetype, SchemaChange::Created) => {
                SchemaEventKind::ArchetypeCreated
            }
            (SchemaKindFamily::Archetype, SchemaChange::Updated) => {
                SchemaEventKind::ArchetypeUpdated
            }
            (SchemaKindFamily::Archetype, SchemaChange::Deleted) => {
                SchemaEventKind::ArchetypeDeleted
            }
            (SchemaKindFamily::ElementType, SchemaChange::Created) => {
                SchemaEventKind::ElementTypeCreated
            }
            (SchemaKindFamily::ElementType, SchemaChange::Updated) => {
                SchemaEventKind::ElementTypeUpdated
            }
            (SchemaKindFamily::ElementType, SchemaChange::Deleted) => {
                SchemaEventKind::ElementTypeDeleted
            }
        }
    }
}

/// Publish an `Event::Schema` for a schema change made on THIS node.
///
/// `schema_id` is the schema's NAME, matching what the replication applicator
/// puts there. A `None` bus is a no-op, so repositories constructed without one
/// (tests, embedded uses, the compound-index job) behave exactly as before.
pub(crate) fn publish_local_schema_event(
    event_bus: Option<&Arc<dyn EventBus>>,
    scope: BranchScope<'_>,
    schema_id: &str,
    family: SchemaKindFamily,
    change: SchemaChange,
) {
    let Some(bus) = event_bus else {
        return;
    };

    let kind = family.event_kind(change);

    let mut metadata = HashMap::new();
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String("local".to_string()),
    );

    tracing::debug!(
        schema_id = %schema_id,
        schema_type = %family.schema_type(),
        kind = ?kind,
        source = "local",
        "Emitting schema event for local operation"
    );

    bus.publish(Event::Schema(SchemaEvent {
        tenant_id: scope.tenant_id.to_string(),
        repository_id: scope.repo_id.to_string(),
        branch: scope.branch.to_string(),
        schema_id: schema_id.to_string(),
        schema_type: family.schema_type().to_string(),
        kind,
        metadata: Some(metadata),
    }))
}
