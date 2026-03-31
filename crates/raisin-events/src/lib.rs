// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Event system for RaisinDB
//!
//! This crate provides event types and event bus infrastructure for
//! building observable, event-driven systems in RaisinDB.

mod bus;

pub use bus::InMemoryEventBus;

use anyhow::Result;
use raisin_hlc::HLC;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Top-level event type that wraps all event kinds
#[derive(Debug, Clone)]
pub enum Event {
    /// Repository-level event (creation, deletion, branches, tags, commits)
    Repository(RepositoryEvent),
    /// Workspace-level event (workspace lifecycle)
    Workspace(WorkspaceEvent),
    /// Node-level event (individual node CRUD)
    Node(NodeEvent),
    /// Replication-level event (operation batches, sync status)
    Replication(ReplicationEvent),
    /// Schema-level event (NodeType, Archetype, ElementType changes)
    Schema(SchemaEvent),
}

/// Event filter for subscribing to specific event types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventFilter {
    /// Match all events
    All,
    /// Match all repository events
    AllRepository,
    /// Match specific repository event kind
    Repository(RepositoryEventKind),
    /// Match all workspace events
    AllWorkspace,
    /// Match specific workspace event kind
    Workspace(WorkspaceEventKind),
    /// Match all node events
    AllNode,
    /// Match specific node event kind
    Node(NodeEventKind),
    /// Match all replication events
    AllReplication,
    /// Match specific replication event kind
    Replication(ReplicationEventKind),
    /// Match all schema events
    AllSchema,
    /// Match specific schema event kind
    Schema(SchemaEventKind),
}

impl EventFilter {
    /// Check if this filter matches the given event
    pub fn matches(&self, event: &Event) -> bool {
        match (self, event) {
            (EventFilter::All, _) => true,
            (EventFilter::AllRepository, Event::Repository(_)) => true,
            (EventFilter::Repository(kind), Event::Repository(evt)) => &evt.kind == kind,
            (EventFilter::AllWorkspace, Event::Workspace(_)) => true,
            (EventFilter::Workspace(kind), Event::Workspace(evt)) => &evt.kind == kind,
            (EventFilter::AllNode, Event::Node(_)) => true,
            (EventFilter::Node(kind), Event::Node(evt)) => &evt.kind == kind,
            (EventFilter::AllReplication, Event::Replication(_)) => true,
            (EventFilter::Replication(kind), Event::Replication(evt)) => &evt.kind == kind,
            (EventFilter::AllSchema, Event::Schema(_)) => true,
            (EventFilter::Schema(kind), Event::Schema(evt)) => &evt.kind == kind,
            _ => false,
        }
    }
}

/// Kind of node lifecycle event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEventKind {
    /// Node was created
    Created,
    /// Node was updated
    Updated,
    /// Node was deleted
    Deleted,
    /// Node was reordered among siblings
    Reordered,
    /// Node was published
    Published,
    /// Node was unpublished
    Unpublished,
    /// Single property was changed
    PropertyChanged { property: String },
    /// Relationship was added to a node
    RelationAdded {
        relation_type: String,
        target_node_id: String,
    },
    /// Relationship was removed from a node
    RelationRemoved {
        relation_type: String,
        target_node_id: String,
    },
}

/// A node lifecycle event
#[derive(Debug, Clone)]
pub struct NodeEvent {
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repository_id: String,
    /// Branch name
    pub branch: String,
    /// Workspace ID
    pub workspace_id: String,
    /// Node ID
    pub node_id: String,
    /// Node type (optional)
    pub node_type: Option<String>,
    /// Revision when this event occurred
    pub revision: HLC,
    /// Kind of event
    pub kind: NodeEventKind,
    /// Node path (optional)
    pub path: Option<String>,
    /// Additional metadata (optional)
    pub metadata: Option<HashMap<String, JsonValue>>,
}

/// Kind of repository-level event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryEventKind {
    /// Tenant was registered for the first time
    TenantCreated,
    /// Repository was created
    Created,
    /// Repository was updated
    Updated,
    /// Repository was deleted
    Deleted,
    /// Commit was created (affects repository history)
    CommitCreated,
    /// Branch was created
    BranchCreated,
    /// Branch HEAD was updated
    BranchUpdated,
    /// Branch was deleted
    BranchDeleted,
    /// Tag was created
    TagCreated,
    /// Tag was deleted
    TagDeleted,
}

/// A repository-level event (repository lifecycle and git operations)
#[derive(Debug, Clone)]
pub struct RepositoryEvent {
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repository_id: String,
    /// Kind of event
    pub kind: RepositoryEventKind,
    /// Workspace ID (for commits that happen in a workspace context)
    pub workspace: Option<String>,
    /// Revision ID (for commits)
    pub revision_id: Option<String>,
    /// Branch name (for branch operations)
    pub branch_name: Option<String>,
    /// Tag name (for tag operations)
    pub tag_name: Option<String>,
    /// Commit message (for commits)
    pub message: Option<String>,
    /// Actor who performed the operation
    pub actor: Option<String>,
    /// Additional metadata (optional)
    pub metadata: Option<HashMap<String, JsonValue>>,
}

/// Kind of workspace-level event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceEventKind {
    /// Workspace was created
    Created,
    /// Workspace was updated
    Updated,
    /// Workspace was deleted
    Deleted,
}

/// A workspace-level event
#[derive(Debug, Clone)]
pub struct WorkspaceEvent {
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repository_id: String,
    /// Workspace ID
    pub workspace: String,
    /// Kind of event
    pub kind: WorkspaceEventKind,
    /// Additional metadata (optional)
    pub metadata: Option<HashMap<String, JsonValue>>,
}

/// Kind of replication-level event
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationEventKind {
    /// Operation batch was applied during replication catch-up
    OperationBatchApplied,
}

/// A replication-level event
#[derive(Debug, Clone)]
pub struct ReplicationEvent {
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repository_id: String,
    /// Branch name (optional, may be multi-branch sync)
    pub branch: Option<String>,
    /// Workspace ID (optional, may be multi-workspace sync)
    pub workspace: Option<String>,
    /// Number of operations in the batch
    pub operation_count: usize,
    /// Kind of event
    pub kind: ReplicationEventKind,
    /// Additional metadata (optional)
    pub metadata: Option<HashMap<String, JsonValue>>,
}

/// Kind of schema-level event (NodeType, Archetype, ElementType changes)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaEventKind {
    /// NodeType was created
    NodeTypeCreated,
    /// NodeType was updated
    NodeTypeUpdated,
    /// NodeType was deleted
    NodeTypeDeleted,
    /// Archetype was created
    ArchetypeCreated,
    /// Archetype was updated
    ArchetypeUpdated,
    /// Archetype was deleted
    ArchetypeDeleted,
    /// ElementType was created
    ElementTypeCreated,
    /// ElementType was updated
    ElementTypeUpdated,
    /// ElementType was deleted
    ElementTypeDeleted,
}

/// A schema-level event (NodeType, Archetype, or ElementType change)
#[derive(Debug, Clone)]
pub struct SchemaEvent {
    /// Tenant ID
    pub tenant_id: String,
    /// Repository ID
    pub repository_id: String,
    /// Branch name
    pub branch: String,
    /// Schema ID (ID of the NodeType, Archetype, or ElementType)
    pub schema_id: String,
    /// Schema type ("NodeType", "Archetype", or "ElementType")
    pub schema_type: String,
    /// Kind of event
    pub kind: SchemaEventKind,
    /// Additional metadata (optional, includes source: local/replication)
    pub metadata: Option<HashMap<String, JsonValue>>,
}

/// Event handler trait for processing events
///
/// Implement this trait to create custom event handlers that react to
/// node, repository, or git events.
pub trait EventHandler: Send + Sync {
    /// Handle an event asynchronously
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    /// Get the handler name (for logging)
    fn name(&self) -> &str;
}

/// Wrapper for closure-based event handlers
///
/// This allows subscribing to events with closures instead of implementing
/// the full EventHandler trait. Optionally filter which events to handle.
pub struct FnEventHandler<F>
where
    F: Fn(&Event) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> + Send + Sync,
{
    handler: F,
    name: String,
    filter: EventFilter,
}

impl<F> FnEventHandler<F>
where
    F: Fn(&Event) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> + Send + Sync,
{
    /// Create a new function-based event handler with a filter
    pub fn new(name: impl Into<String>, filter: EventFilter, handler: F) -> Self {
        Self {
            handler,
            name: name.into(),
            filter,
        }
    }
}

impl<F> EventHandler for FnEventHandler<F>
where
    F: Fn(&Event) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> + Send + Sync,
{
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        if self.filter.matches(event) {
            (self.handler)(event)
        } else {
            Box::pin(async { Ok(()) })
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Event bus trait for publishing and subscribing to events
pub trait EventBus: Send + Sync {
    /// Publish an event to all subscribers
    fn publish(&self, event: Event);

    /// Subscribe a handler to receive events
    fn subscribe(&self, handler: Arc<dyn EventHandler>);

    /// Clear all subscribers
    fn clear_subscribers(&self);
}

/// Extension methods for EventBus
pub trait EventBusExt {
    /// Subscribe a closure-based handler with an event filter
    ///
    /// # Example
    /// ```no_run
    /// # use raisin_events::{InMemoryEventBus, EventBusExt, Event, EventFilter, RepositoryEventKind};
    /// let bus = InMemoryEventBus::new();
    ///
    /// // Subscribe to specific event type
    /// bus.subscribe_fn("logger", EventFilter::Repository(RepositoryEventKind::Created), |event| {
    ///     Box::pin(async move {
    ///         println!("Repository created: {:?}", event);
    ///         Ok(())
    ///     })
    /// });
    ///
    /// // Subscribe to all repository events
    /// bus.subscribe_fn("audit", EventFilter::AllRepository, |event| {
    ///     Box::pin(async move {
    ///         println!("Repository event: {:?}", event);
    ///         Ok(())
    ///     })
    /// });
    ///
    /// // Subscribe to all events
    /// bus.subscribe_fn("metrics", EventFilter::All, |event| {
    ///     Box::pin(async move {
    ///         println!("Event: {:?}", event);
    ///         Ok(())
    ///     })
    /// });
    /// ```
    fn subscribe_fn<F>(&self, name: impl Into<String>, filter: EventFilter, handler: F)
    where
        F: Fn(&Event) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>
            + Send
            + Sync
            + 'static;
}

impl<T: EventBus + ?Sized> EventBusExt for T {
    fn subscribe_fn<F>(&self, name: impl Into<String>, filter: EventFilter, handler: F)
    where
        F: Fn(&Event) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>
            + Send
            + Sync
            + 'static,
    {
        self.subscribe(Arc::new(FnEventHandler::new(name, filter, handler)));
    }
}
