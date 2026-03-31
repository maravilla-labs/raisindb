# Core Services API

Reference documentation for core RaisinDB services.

## NodeService

The main service for node CRUD operations. NodeService is scoped to a specific tenant, repository, branch, and workspace.

### Construction

The recommended way to create a NodeService is through the connection API:

```rust
use raisin_core::connection::RaisinConnection;

let conn = RaisinConnection::with_storage(storage);
let service = conn
    .tenant("my-tenant")
    .repository("my-repo")
    .workspace("content")
    .nodes();
```

Alternatively, use `new_with_context` directly:

```rust
let service = NodeService::new_with_context(
    storage,
    "my-tenant".to_string(),
    "my-repo".to_string(),
    "main".to_string(),
    "content".to_string(),
);
```

> **Note**: `NodeService::new(storage)` still exists but is deprecated. It creates a single-tenant service with default scope (`default/default/main/default`).

### Methods

Node CRUD operations are delegated to the underlying `NodeRepository` trait, which uses `StorageScope` for multi-tenant isolation. The NodeService adds validation, audit logging, and business logic on top.

#### `get(&self, id: &str) -> Result<Option<Node>>`

Get a node by ID.

#### `get_by_path(&self, path: &str) -> Result<Option<Node>>`

Get a node by path.

#### `add_node(&self, parent_path: &str, node: Node) -> Result<Node>`

Create a new node with validation.

**Parameters:**
- `parent_path`: Parent location (e.g., "/", "/parent")
- `node`: Node data with name, node_type, properties

**Returns:** Created node with generated ID and path

#### `update(&self, node: Node) -> Result<()>`

Update an existing node.

#### `delete(&self, id: &str) -> Result<bool>`

Delete a node by ID.

#### `list_children(&self, parent_path: &str) -> Result<Vec<Node>>`

List children of a parent node.

#### Tree Operations

- `move_node(id, new_path)` - Move a node
- `rename_node(old_path, new_name)` - Rename a node
- `copy_node(source_path, target_parent, new_name)` - Copy a node
- `copy_node_tree(source_path, target_parent, new_name)` - Copy a node tree
- `publish(node_path)` / `unpublish(node_path)` - Publishing workflow
- `publish_tree(node_path)` / `unpublish_tree(node_path)` - Tree publishing

#### Configuration

```rust
// Add audit logging
let service = service.with_audit(audit_adapter);

// Add authentication context for RLS
let service = service.with_auth(auth_context);
```

See the [source code](../../crates/raisin-core/src/services/node_service/) for complete API.

## WorkspaceService

Service for workspace management. WorkspaceService takes tenant and repository parameters for each operation.

### Constructor

```rust
let ws_service = WorkspaceService::new(storage);
```

### Methods

#### `get(&self, tenant_id: &str, repo_id: &str, name: &str) -> Result<Option<Workspace>>`

Get a workspace by name.

#### `put(&self, tenant_id: &str, repo_id: &str, ws: Workspace) -> Result<()>`

Create or update a workspace. When creating a new workspace, this automatically:
- Creates a ROOT node at path `/`
- Initializes the workspace's initial structure (if configured)

#### `list(&self, tenant_id: &str, repo_id: &str) -> Result<Vec<Workspace>>`

List all workspaces in a repository.

## ReferenceResolver

Service for resolving node references automatically.

### Constructor

#### `new(storage: Arc<S>, tenant_id: String, repo_id: String, branch: String) -> Self`

Create a new ReferenceResolver scoped to a tenant, repository, and branch.

```rust
use raisin_core::services::reference_resolver::ReferenceResolver;
use std::sync::Arc;

let resolver = ReferenceResolver::new(
    storage,
    "my-tenant".to_string(),
    "my-repo".to_string(),
    "main".to_string(),
);
```

### Methods

#### `resolve(&self, workspace: &str, node: &Node) -> Result<ResolvedNode>`

Resolve all references in a node and return them in a map.

**Parameters:**
- `workspace`: Workspace name
- `node`: Node containing references to resolve

**Returns:** `ResolvedNode` containing the original node and a map of resolved references

**Example:**
```rust
let node = service.get("content", "article-1").await?.unwrap();
let resolved = resolver.resolve("content", &node).await?;

// Access original node
println!("Article: {}", resolved.node.name);

// Access resolved references
for (ref_id, ref_node) in resolved.resolved_references {
    println!("Referenced: {} ({})", ref_node.name, ref_id);
}
```

**ResolvedNode Structure:**
```rust
pub struct ResolvedNode {
    /// The original node (unchanged)
    pub node: Node,
    /// Map of reference ID → resolved node
    pub resolved_references: HashMap<String, Node>,
}
```

#### `resolve_inline(&self, workspace: &str, node: &Node) -> Result<Node>`

Resolve references and replace them inline with full node objects.

**Parameters:**
- `workspace`: Workspace name
- `node`: Node containing references to resolve

**Returns:** New node with references replaced by `PropertyValue::Object` containing full node data

**Example:**
```rust
let node = service.get("content", "article-1").await?.unwrap();

// Before: properties["author"] = Reference({ id: "user-123", ... })
let resolved = resolver.resolve_inline("content", &node).await?;
// After: properties["author"] = Object({ id: "user-123", name: "John", ... })

// Access resolved data
if let Some(PropertyValue::Object(author)) = resolved.properties.get("author") {
    if let Some(PropertyValue::String(name)) = author.get("name") {
        println!("Author: {}", name);
    }
}
```

**Features:**
- Automatically deduplicates references
- Handles nested references in arrays and objects
- Gracefully handles missing references
- Performs shallow resolution (does not recursively resolve)

See the [Reference Resolution Guide](../guides/reference-resolution.md) for complete documentation and use cases.
