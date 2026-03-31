# Storage Traits

Reference for the storage abstraction layer.

## Storage Trait

Main storage interface. The Storage trait provides access to all repository types through associated types.

```rust
pub trait Storage: Send + Sync {
    type Tx: Transaction;
    type Nodes: NodeRepository;
    type NodeTypes: NodeTypeRepository;
    type Archetypes: ArchetypeRepository;
    type ElementTypes: ElementTypeRepository;
    type Workspaces: WorkspaceRepository;
    type Registry: RegistryRepository;
    type PropertyIndex: PropertyIndexRepository;
    type ReferenceIndex: ReferenceIndexRepository;
    type Versioning: VersioningRepository;
    type RepositoryManagement: RepositoryManagementRepository;
    type Branches: BranchRepository;
    type Tags: TagRepository;
    type Revisions: RevisionRepository;
    type GarbageCollection: GarbageCollectionRepository;
    type Trees: TreeRepository;
    type Relations: RelationRepository;
    type Translations: TranslationRepository + Clone;
    type FullTextJobStore: FullTextJobStore;
    type SpatialIndex: SpatialIndexRepository;
    type CompoundIndex: CompoundIndexRepository;

    fn nodes(&self) -> &Self::Nodes;
    fn node_types(&self) -> &Self::NodeTypes;
    fn archetypes(&self) -> &Self::Archetypes;
    fn element_types(&self) -> &Self::ElementTypes;
    fn workspaces(&self) -> &Self::Workspaces;
    fn registry(&self) -> &Self::Registry;
    fn property_index(&self) -> &Self::PropertyIndex;
    fn reference_index(&self) -> &Self::ReferenceIndex;
    fn versioning(&self) -> &Self::Versioning;
    fn branches(&self) -> &Self::Branches;
    fn tags(&self) -> &Self::Tags;
    fn revisions(&self) -> &Self::Revisions;
    fn relations(&self) -> &Self::Relations;
    fn translations(&self) -> &Self::Translations;
    fn begin(&self) -> impl Future<Output = Result<Self::Tx>> + Send;
    fn event_bus(&self) -> Arc<dyn EventBus>;
    // ... workspace delta operations
}
```

### Implementations

- `RocksStorage` - RocksDB backend (persistent)
- `InMemoryStorage` - In-memory backend (testing)

## NodeRepository Trait

Node storage operations. All methods take a `StorageScope` parameter that bundles tenant, repository, branch, and workspace context.

```rust
pub trait NodeRepository: Send + Sync {
    // Core CRUD
    fn get(&self, scope: StorageScope<'_>, id: &str, max_revision: Option<&HLC>)
        -> impl Future<Output = Result<Option<Node>>> + Send;
    fn create(&self, scope: StorageScope<'_>, node: Node, options: CreateNodeOptions)
        -> impl Future<Output = Result<()>> + Send;
    fn update(&self, scope: StorageScope<'_>, node: Node, options: UpdateNodeOptions)
        -> impl Future<Output = Result<()>> + Send;
    fn delete(&self, scope: StorageScope<'_>, id: &str, options: DeleteNodeOptions)
        -> impl Future<Output = Result<bool>> + Send;

    // List operations (with performance controls)
    fn list_all(&self, scope: StorageScope<'_>, options: ListOptions)
        -> impl Future<Output = Result<Vec<Node>>> + Send;
    fn list_by_type(&self, scope: StorageScope<'_>, node_type: &str, options: ListOptions)
        -> impl Future<Output = Result<Vec<Node>>> + Send;
    fn list_children(&self, scope: StorageScope<'_>, parent_path: &str, options: ListOptions)
        -> impl Future<Output = Result<Vec<Node>>> + Send;

    // Path-based operations
    fn get_by_path(&self, scope: StorageScope<'_>, path: &str, max_revision: Option<&HLC>)
        -> impl Future<Output = Result<Option<Node>>> + Send;

    // Tree operations
    fn move_node(&self, scope: StorageScope<'_>, id: &str, new_path: &str, ...)
        -> impl Future<Output = Result<()>> + Send;
    fn rename_node(&self, scope: StorageScope<'_>, old_path: &str, new_name: &str)
        -> impl Future<Output = Result<()>> + Send;
    fn copy_node(&self, scope: StorageScope<'_>, source_path: &str, target_parent: &str, ...)
        -> impl Future<Output = Result<Node>> + Send;

    // Publish/unpublish
    fn publish(&self, scope: StorageScope<'_>, node_path: &str)
        -> impl Future<Output = Result<()>> + Send;
    fn unpublish(&self, scope: StorageScope<'_>, node_path: &str)
        -> impl Future<Output = Result<()>> + Send;

    // ... and many more (deep traversal, reordering, property access, etc.)
}
```

The `StorageScope` bundles:
```rust
StorageScope::new(tenant_id, repo_id, branch, workspace)
```

## NodeTypeRepository Trait

NodeType storage operations. NodeTypes are scoped by `BranchScope` (tenant + repo + branch), allowing each branch to evolve its schemas independently.

```rust
pub trait NodeTypeRepository: Send + Sync {
    fn get(&self, scope: BranchScope<'_>, name: &str, max_revision: Option<&HLC>)
        -> impl Future<Output = Result<Option<NodeType>>> + Send;
    fn create(&self, scope: BranchScope<'_>, node_type: NodeType, commit: CommitMetadata)
        -> impl Future<Output = Result<HLC>> + Send;
    fn update(&self, scope: BranchScope<'_>, node_type: NodeType, commit: CommitMetadata)
        -> impl Future<Output = Result<HLC>> + Send;
    fn upsert(&self, scope: BranchScope<'_>, node_type: NodeType, commit: CommitMetadata)
        -> impl Future<Output = Result<HLC>> + Send;
    fn delete(&self, scope: BranchScope<'_>, name: &str, commit: CommitMetadata)
        -> impl Future<Output = Result<Option<HLC>>> + Send;
    fn list(&self, scope: BranchScope<'_>, max_revision: Option<&HLC>)
        -> impl Future<Output = Result<Vec<NodeType>>> + Send;
    fn list_published(&self, scope: BranchScope<'_>, max_revision: Option<&HLC>)
        -> impl Future<Output = Result<Vec<NodeType>>> + Send;
    fn publish(&self, scope: BranchScope<'_>, name: &str, commit: CommitMetadata)
        -> impl Future<Output = Result<HLC>> + Send;
    fn unpublish(&self, scope: BranchScope<'_>, name: &str, commit: CommitMetadata)
        -> impl Future<Output = Result<HLC>> + Send;
    // ... batch operations, version resolution, etc.
}
```

The `BranchScope` bundles:
```rust
BranchScope::new(tenant_id, repo_id, branch)
```

## WorkspaceRepository Trait

Workspace storage operations. Workspaces are scoped by `RepoScope` (tenant + repo).

```rust
pub trait WorkspaceRepository: Send + Sync {
    fn get(&self, scope: RepoScope<'_>, id: &str)
        -> impl Future<Output = Result<Option<Workspace>>> + Send;
    fn put(&self, scope: RepoScope<'_>, ws: Workspace)
        -> impl Future<Output = Result<()>> + Send;
    fn list(&self, scope: RepoScope<'_>)
        -> impl Future<Output = Result<Vec<Workspace>>> + Send;
}
```

The `RepoScope` bundles:
```rust
RepoScope::new(tenant_id, repo_id)
```

## VersioningRepository Trait

Tracks node version history. Versioning methods are scoped per-node (identified by node_id).

```rust
pub trait VersioningRepository: Send + Sync {
    fn create_version(&self, node: &Node) -> impl Future<Output = Result<i32>> + Send;
    fn create_version_with_note(&self, node: &Node, note: Option<String>)
        -> impl Future<Output = Result<i32>> + Send;
    fn list_versions(&self, node_id: &str)
        -> impl Future<Output = Result<Vec<NodeVersion>>> + Send;
    fn get_version(&self, node_id: &str, version: i32)
        -> impl Future<Output = Result<Option<NodeVersion>>> + Send;
    fn delete_version(&self, node_id: &str, version: i32)
        -> impl Future<Output = Result<bool>> + Send;
    fn delete_all_versions(&self, node_id: &str)
        -> impl Future<Output = Result<usize>> + Send;
    fn delete_old_versions(&self, node_id: &str, keep_count: usize)
        -> impl Future<Output = Result<usize>> + Send;
    fn update_version_note(&self, node_id: &str, version: i32, note: Option<String>)
        -> impl Future<Output = Result<()>> + Send;
}
```

## ReferenceIndexRepository Trait

Bidirectional index for node references.

### Overview

The `ReferenceIndexRepository` maintains automatic indexes for `PropertyValue::Reference` instances, enabling fast bidirectional lookups:

- **Forward Index**: Source node → list of referenced nodes
- **Reverse Index**: Target node → list of nodes referencing it

References are indexed automatically during `put`, `delete`, `publish`, and `unpublish` operations. No manual indexing is required.

### Trait Definition

All methods take a `StorageScope` (tenant + repo + branch + workspace) and use MVCC revisions via `HLC` timestamps.

```rust
pub trait ReferenceIndexRepository: Send + Sync {
    /// Index all references in a node's properties
    fn index_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &HLC,
        is_published: bool,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Remove all reference indexes for a node (writes tombstones for MVCC)
    fn unindex_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &HLC,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Update publish status for a node's reference indexes
    fn update_reference_publish_status(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        properties: &HashMap<String, PropertyValue>,
        revision: &HLC,
        is_published: bool,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Find all nodes that reference a specific target (reverse lookup)
    /// Returns Vec<(source_node_id, property_path)>
    fn find_referencing_nodes(
        &self,
        scope: StorageScope<'_>,
        target_workspace: &str,
        target_path: &str,
        published_only: bool,
    ) -> impl Future<Output = Result<Vec<(String, String)>>> + Send;

    /// Get all references from a specific node (forward lookup)
    /// Returns Vec<(property_path, RaisinReference)>
    fn get_node_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> impl Future<Output = Result<Vec<(String, RaisinReference)>>> + Send;

    /// Get unique target references from a node (deduplicated)
    fn get_unique_references(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        published_only: bool,
    ) -> impl Future<Output = Result<HashMap<String, (Vec<String>, RaisinReference)>>> + Send;
}
```

### Methods

#### `index_references()`

Index all references from a node. Called automatically during `create()` and `update()` operations.

**Parameters:**
- `scope`: Storage scope (tenant + repo + branch + workspace)
- `node_id`: Source node ID
- `properties`: Node properties containing references
- `revision`: HLC timestamp for MVCC
- `is_published`: Whether to index in published or draft space

#### `unindex_references()`

Remove all reference indexes for a node by writing tombstones (MVCC). Called automatically during `delete()` operations.

**Parameters:**
- `scope`: Storage scope
- `node_id`: Node ID to unindex
- `properties`: Node properties (needed to find references to tombstone)
- `revision`: HLC timestamp for the tombstone

#### `find_referencing_nodes()`

Find all nodes that reference a specific target. This is the **reverse lookup** - given a target workspace and path, find who references it.

**Parameters:**
- `scope`: Storage scope
- `target_workspace`: Target workspace name
- `target_path`: Target node path
- `published_only`: Query published (true) or all (false) references

**Returns:** Vector of `(source_node_id, property_path)` tuples

**Example:**
```rust
let scope = StorageScope::new("tenant-1", "myapp", "main", "content");

// Find all articles that reference the author at /authors/john
let referencing = storage.reference_index()
    .find_referencing_nodes(scope, "content", "/authors/john", true)
    .await?;

for (source_id, prop_path) in &referencing {
    println!("Node {} references via property '{}'", source_id, prop_path);
}
```

#### `get_node_references()`

Get all references from a specific node. This is the **forward lookup**.

**Parameters:**
- `scope`: Storage scope
- `node_id`: Source node ID
- `published_only`: Query published (true) or all (false) references

**Returns:** Vector of `(property_path, RaisinReference)` tuples

**Example:**
```rust
let refs = storage.reference_index()
    .get_node_references(scope, "article-1", false)
    .await?;

for (prop_path, raisin_ref) in &refs {
    println!("Property '{}' references {}:{}", prop_path, raisin_ref.workspace, raisin_ref.path);
}
```

#### `get_unique_references()`

Get deduplicated target references from a node, grouping multiple property paths that reference the same target.

#### `update_reference_publish_status()`

Move references between draft and published indexes. Called automatically during `publish()` and `unpublish()` operations.

### Draft vs Published Indexes

References are maintained in separate index spaces for draft and published content. RocksDB uses `ref`/`ref_rev` prefixes for draft and `ref_pub`/`ref_rev_pub` for published.

### Performance

- **O(log n) lookup** for both forward and reverse queries (RocksDB prefix scan)
- **Automatic deduplication** via `get_unique_references()`
- **MVCC-aware** - tombstones enable time-travel queries
- **Compact encoding** - efficient key structure in RocksDB
- **Concurrent access** - Thread-safe via native RocksDB concurrency

See the [Reference Property Type](../guides/property-reference.md#reference-indexing) for usage patterns.
