# Event Emission Guide

## Quick Reference

This guide shows how to emit events from different parts of RaisinDB.

## Repository Events

Repository events are emitted from the storage layer in `RepositoryManagementRepository` implementations.

### Example: Repository Created (Already Implemented)

```rust
// In InMemoryRepositoryManagement::create_repository()
async fn create_repository(&self, tenant_id: &str, repo_id: &str, config: RepositoryConfig) 
    -> Result<RepositoryInfo> 
{
    // ... create repository ...
    
    // Emit event
    let event = Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::Created,
        branch_name: Some(config.default_branch.clone()),
        workspace: None,
        revision_id: None,
        tag_name: None,
        message: None,
        actor: None,
        metadata: None,
    });
    self.event_bus.publish(event);
    
    Ok(info)
}
```

### Pattern: Add to Other Repository Operations

To add events to update/delete operations:

```rust
// RepositoryUpdated
async fn update_repository_config(&self, tenant_id: &str, repo_id: &str, config: RepositoryConfig) 
    -> Result<()> 
{
    // ... update config ...
    
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::Updated,
        branch_name: None,
        workspace: None,
        revision_id: None,
        tag_name: None,
        message: None,
        actor: None,
        metadata: Some({
            let mut meta = HashMap::new();
            meta.insert("config".to_string(), serde_json::to_value(&config)?);
            meta
        }),
    }));
    
    Ok(())
}

// RepositoryDeleted
async fn delete_repository(&self, tenant_id: &str, repo_id: &str) -> Result<bool> {
    let existed = // ... delete repository ...;
    
    if existed {
        self.event_bus.publish(Event::Repository(RepositoryEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            kind: RepositoryEventKind::Deleted,
            branch_name: None,
            workspace: None,
            revision_id: None,
            tag_name: None,
            message: None,
            actor: None,
            metadata: None,
        }));
    }
    
    Ok(existed)
}
```

## Node Events

Node events should be emitted from `NodeService` or HTTP handlers after successful operations.

### Pattern: Emit from HTTP Handlers

The cleanest approach is to emit events from HTTP handlers after successful operations:

```rust
// In crates/raisin-transport-http/src/handlers/repo.rs

pub async fn create_node(
    State(state): State<AppState>,
    Path((repo, branch, workspace, path)): Path<(String, String, String, String)>,
    Json(req): Json<CreateNodeRequest>,
) -> Response {
    // ... create node via NodeService ...
    
    match node_service.create(...).await {
        Ok(node) => {
            // Emit NodeCreated event
            state.storage().event_bus().publish(Event::Node(NodeEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo.clone(),
                branch: branch.clone(),
                node_id: node.id.clone(),
                node_type: Some(req.node_type.clone()),
                kind: NodeEventKind::Created,
                path: Some(path.clone()),
                metadata: Some({
                    let mut meta = HashMap::new();
                    meta.insert("properties".to_string(), serde_json::to_value(&node.properties)?);
                    meta
                }),
            }));
            
            (StatusCode::CREATED, Json(node)).into_response()
        }
        Err(e) => // ... error handling
    }
}
```

### Pattern: Emit from NodeService

Alternatively, add event_bus to NodeService:

```rust
// In crates/raisin-core/src/services/node_service/mod.rs

pub struct NodeService<S: Storage> {
    storage: Arc<S>,
    event_bus: Arc<dyn EventBus>,  // ADD THIS
    // ... other fields
}

impl<S: Storage> NodeService<S> {
    pub async fn create_node(&self, ...) -> Result<Node> {
        // ... create node ...
        
        // Emit event
        self.event_bus.publish(Event::Node(NodeEvent {
            tenant_id: self.tenant_id.clone(),
            repository_id: self.repo_id.clone(),
            branch: self.branch.clone(),
            node_id: node.id.clone(),
            node_type: Some(node.node_type.clone()),
            kind: NodeEventKind::Created,
            path: Some(path),
            metadata: Some({
                let mut meta = HashMap::new();
                meta.insert("properties".to_string(), serde_json::to_value(&node.properties)?);
                meta
            }),
        }));
        
        Ok(node)
    }
    
    pub async fn update_node(&self, node_id: &str, updates: UpdateNodeRequest) -> Result<Node> {
        // ... update node ...
        
        self.event_bus.publish(Event::Node(NodeEvent {
            tenant_id: self.tenant_id.clone(),
            repository_id: self.repo_id.clone(),
            branch: self.branch.clone(),
            node_id: node_id.to_string(),
            node_type: Some(node.node_type.clone()),
            kind: NodeEventKind::Updated,
            path: None,
            metadata: Some({
                let mut meta = HashMap::new();
                meta.insert("properties".to_string(), serde_json::to_value(&node.properties)?);
                meta
            }),
        }));
        
        Ok(node)
    }
    
    pub async fn delete_node(&self, node_id: &str) -> Result<()> {
        // ... delete node ...
        
        self.event_bus.publish(Event::Node(NodeEvent {
            tenant_id: self.tenant_id.clone(),
            repository_id: self.repo_id.clone(),
            branch: self.branch.clone(),
            node_id: node_id.to_string(),
            node_type: None,
            kind: NodeEventKind::Deleted,
            path: None,
            metadata: None,
        }));
        
        Ok(())
    }
}
```

## Transaction Events (Commits)

Emit `CommitCreated` events when transactions are committed:

```rust
// In TransactionService or wherever commits happen

pub async fn commit_transaction(&self, workspace: &str, message: &str, actor: Option<&str>) 
    -> Result<u64> 
{
    // ... commit transaction ...
    let revision_id = // ... created revision ID ...;
    
    // Emit CommitCreated event
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: self.tenant_id.clone(),
        repository_id: self.repo_id.clone(),
        kind: RepositoryEventKind::CommitCreated,
        workspace: Some(workspace.to_string()),
        revision_id: Some(revision_id.to_string()),
        branch_name: Some(self.branch.clone()),
        tag_name: None,
        message: Some(message.to_string()),
        actor: actor.map(|s| s.to_string()),
        metadata: None,
    }));
    
    Ok(revision_id)
}
```

## Branch Events

Emit from `BranchRepository` implementations:

```rust
// BranchCreated
async fn create_branch(&self, tenant_id: &str, repo_id: &str, branch_name: &str, from_revision: u64) 
    -> Result<()> 
{
    // ... create branch ...
    
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::BranchCreated,
        workspace: None,
        revision_id: Some(from_revision.to_string()),
        branch_name: Some(branch_name.to_string()),
        tag_name: None,
        message: None,
        actor: None,
        metadata: None,
    }));
    
    Ok(())
}

// BranchUpdated (when HEAD changes)
async fn update_branch_head(&self, tenant_id: &str, repo_id: &str, branch_name: &str, new_revision: u64) 
    -> Result<()> 
{
    // ... update branch HEAD ...
    
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::BranchUpdated,
        workspace: None,
        revision_id: Some(new_revision.to_string()),
        branch_name: Some(branch_name.to_string()),
        tag_name: None,
        message: None,
        actor: None,
        metadata: None,
    }));
    
    Ok(())
}

// BranchDeleted
async fn delete_branch(&self, tenant_id: &str, repo_id: &str, branch_name: &str) 
    -> Result<()> 
{
    // ... delete branch ...
    
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::BranchDeleted,
        workspace: None,
        revision_id: None,
        branch_name: Some(branch_name.to_string()),
        tag_name: None,
        message: None,
        actor: None,
        metadata: None,
    }));
    
    Ok(())
}
```

## Tag Events

Emit from `TagRepository` implementations:

```rust
// TagCreated
async fn create_tag(&self, tenant_id: &str, repo_id: &str, tag_name: &str, revision: u64, message: Option<&str>) 
    -> Result<()> 
{
    // ... create tag ...
    
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::TagCreated,
        workspace: None,
        revision_id: Some(revision.to_string()),
        branch_name: None,
        tag_name: Some(tag_name.to_string()),
        message: message.map(|s| s.to_string()),
        actor: None,
        metadata: None,
    }));
    
    Ok(())
}

// TagDeleted
async fn delete_tag(&self, tenant_id: &str, repo_id: &str, tag_name: &str) 
    -> Result<()> 
{
    // ... delete tag ...
    
    self.event_bus.publish(Event::Repository(RepositoryEvent {
        tenant_id: tenant_id.to_string(),
        repository_id: repo_id.to_string(),
        kind: RepositoryEventKind::TagDeleted,
        workspace: None,
        revision_id: None,
        branch_name: None,
        tag_name: Some(tag_name.to_string()),
        message: None,
        actor: None,
        metadata: None,
    }));
    
    Ok(())
}
```

## Workspace Events

Emit from `WorkspaceService` or `WorkspaceRepository`:

```rust
// WorkspaceCreated
async fn create_workspace(&self, workspace_id: &str) -> Result<Workspace> {
    // ... create workspace ...
    
    self.event_bus.publish(Event::Workspace(WorkspaceEvent {
        tenant_id: self.tenant_id.clone(),
        repository_id: self.repo_id.clone(),
        workspace: workspace_id.to_string(),
        kind: WorkspaceEventKind::Created,
        metadata: None,
    }));
    
    Ok(workspace)
}

// WorkspaceDeleted
async fn delete_workspace(&self, workspace_id: &str) -> Result<()> {
    // ... delete workspace ...
    
    self.event_bus.publish(Event::Workspace(WorkspaceEvent {
        tenant_id: self.tenant_id.clone(),
        repository_id: self.repo_id.clone(),
        workspace: workspace_id.to_string(),
        kind: WorkspaceEventKind::Deleted,
        metadata: None,
    }));
    
    Ok(())
}
```

## Implementation Checklist

### ✅ Completed
- [x] EventBus infrastructure with closure support
- [x] RepositoryCreated event (from InMemoryRepositoryManagement)
- [x] NodeTypeInitHandler (reacts to RepositoryCreated)
- [x] PropertyIndexPlugin (reacts to Node events)
- [x] Comprehensive mdBook documentation

### ⏳ To Implement
- [ ] RepositoryUpdated event (in update_repository_config)
- [ ] RepositoryDeleted event (in delete_repository)
- [ ] CommitCreated event (in transaction commit)
- [ ] BranchCreated/Updated/Deleted events (in BranchRepository)
- [ ] TagCreated/Deleted events (in TagRepository)
- [ ] NodeCreated/Updated/Deleted events (in NodeService or HTTP handlers)
- [ ] WorkspaceCreated/Updated/Deleted events (in WorkspaceService)

### Implementation Strategy

1. **Storage Layer** (Easiest - already have event_bus)
   - Repository events: Update/Delete
   - Branch events: Create/Update/Delete
   - Tag events: Create/Delete

2. **Service Layer** (Need to add event_bus parameter)
   - Node events: Create/Update/Delete/Publish/Unpublish
   - Workspace events: Create/Update/Delete
   - Transaction events: CommitCreated

3. **Testing**
   - Write unit tests for each event emission
   - Verify events are published with correct data
   - Test that handlers receive and process events

## Example: Full Integration

```rust
// main.rs - Subscribe all handlers
let event_bus = storage.event_bus();

// Repository initialization
event_bus.subscribe(Arc::new(NodeTypeInitHandler::new(storage.clone())));

// Property indexing
let property_index = Arc::new(PropertyIndexPlugin::new());
event_bus.subscribe(property_index.clone());

// Audit logging (closure-based)
event_bus.subscribe_fn("audit-logger", |event| {
    Box::pin(async move {
        match event {
            Event::Repository(e) => {
                tracing::info!("Repository {}: {:?}", e.repository_id, e.kind);
            }
            Event::Node(e) => {
                tracing::info!("Node {}: {:?}", e.node_id, e.kind);
            }
            Event::Workspace(e) => {
                tracing::info!("Workspace {}: {:?}", e.workspace, e.kind);
            }
        }
        Ok(())
    })
});

// Metrics collection (closure-based)
event_bus.subscribe_fn("metrics", |event| {
    Box::pin(async move {
        // Increment counters, update gauges, etc.
        Ok(())
    })
});
```

## See Also

- [Event-Driven Architecture](../architecture/events.md) - Full documentation
- [Custom Handlers](./custom-handlers.md) - Building your own event handlers
- [Testing Events](./testing-events.md) - Testing event-driven systems
