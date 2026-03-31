# Storage Backends

RaisinDB uses a trait-based storage abstraction that allows multiple backend implementations.

## Available Backends

### RocksDB (Default)

**Crate**: `raisin-rocksdb`

High-performance embedded database with:
- Fast writes via LSM tree
- Efficient prefix scans
- Built-in compression
- ACID transactions

**Usage**:
```rust
use raisin_rocksdb::open_db;

let db = open_db("./data")?;
```

**Best For**:
- Production deployments
- Large datasets
- High write throughput
- Multi-tenancy

### InMemory

**Crate**: `raisin-storage-memory`

Fast in-memory storage for:
- Testing
- Development
- Prototyping

**Usage**:
```rust
use raisin_storage_memory::InMemoryStorage;

let storage = InMemoryStorage::default();
```

**Best For**:
- Unit tests
- Integration tests
- Quick prototypes

## Planned Backends

### MongoDB

Document-oriented storage with:
- Flexible schemas
- Native JSON support
- Cloud-ready

### PostgreSQL

Relational database with:
- JSONB support
- Complex queries
- Well-known operations

## Implementing Custom Backends

Implement the `Storage` trait:

```rust
use raisin_storage::Storage;

pub struct MyStorage {
    // Your storage implementation
}

impl Storage for MyStorage {
    type Tx = MyTransaction;
    type Nodes = MyNodeRepo;
    type NodeTypes = MyNodeTypeRepo;
    type Archetypes = MyArchetypeRepo;
    type ElementTypes = MyElementTypeRepo;
    type Workspaces = MyWorkspaceRepo;
    type Registry = MyRegistryRepo;
    type PropertyIndex = MyPropertyIndex;
    type ReferenceIndex = MyReferenceIndex;
    type Versioning = MyVersioningRepo;
    type RepositoryManagement = MyRepoManagement;
    type Branches = MyBranchRepo;
    type Tags = MyTagRepo;
    type Revisions = MyRevisionRepo;
    type GarbageCollection = MyGCRepo;
    type Trees = MyTreeRepo;
    type Relations = MyRelationRepo;
    type Translations = MyTranslationRepo;
    type FullTextJobStore = MyFullTextJobStore;
    type SpatialIndex = MySpatialIndex;
    type CompoundIndex = MyCompoundIndex;

    fn nodes(&self) -> &Self::Nodes { &self.nodes }
    fn node_types(&self) -> &Self::NodeTypes { &self.node_types }
    fn workspaces(&self) -> &Self::Workspaces { &self.workspaces }
    // ... implement all required accessor methods
}
```

See the [Custom Storage Guide](../guides/custom-storage.md) for a complete tutorial.

## Storage Indexes

RaisinDB maintains automatic indexes for fast data access. Indexes are updated synchronously during all CRUD operations.

### Property Index

Enables fast lookups of nodes by property values.

**Capabilities:**
- Find nodes with specific property values
- Query across node types
- Filter by workspace
- Separate draft and published indexes

**Example:**
```rust
use raisin_storage::Storage;

let prop_index = storage.property_index();

// Find all nodes where status = "published"
let nodes = prop_index
    .find_nodes_with_property("content", "status", &PropertyValue::String("published".to_string()), false)
    .await?;
```

**Storage:**
- **InMemory**: HashMap-based in-memory index
- **RocksDB**: Persistent LSM tree with efficient prefix scans

### Reference Index

Enables bidirectional lookups for node references.

**Index Structure:**

1. **Forward Index**: Source node → list of referenced nodes
   ```
   article-1 → [author-123, category-456]
   ```

2. **Reverse Index**: Target node → list of nodes referencing it
   ```
   author-123 → [article-1, article-2, article-3]
   ```

**Capabilities:**
- Find all nodes referencing a target (reverse lookup)
- Find all references from a source node (forward lookup)
- Track property paths where references occur
- Separate draft and published indexes
- Automatic deduplication

**Example:**
```rust
// Find all articles by this author
let article_ids = storage.reference_index()
    .find_nodes_referencing("content", "author-123", true) // true = published only
    .await?;

// Find all references from this article
let references = storage.reference_index()
    .find_outgoing_references("content", "article-1", false) // false = draft
    .await?;

for (target_id, property_paths) in references {
    println!("References {} via: {:?}", target_id, property_paths);
}
```

**Storage:**
- **InMemory**: Dual HashMaps (forward + reverse) with RwLock
- **RocksDB**: Compact key encoding with tenant isolation
  - Forward keys: `ref:ws:source_id:is_pub → [target_id:prop_path, ...]`
  - Reverse keys: `refrev:ws:target_id:is_pub:source_id → 1`

**Performance:**
- O(1) lookup for both forward and reverse queries
- Batch-friendly for multiple references
- Thread-safe concurrent access
- Storage-efficient key encoding

### Draft vs Published Indexes

Both property and reference indexes maintain separate spaces for draft and published content:

```rust
// Query draft index
let draft_results = storage.reference_index()
    .find_nodes_referencing("content", "author-123", false)
    .await?;

// Query published index
let published_results = storage.reference_index()
    .find_nodes_referencing("content", "author-123", true)
    .await?;

// Publishing moves entries from draft to published automatically
service.publish("content", "/article").await?;
```

### Automatic Indexing

Indexes are updated automatically during all CRUD operations:

```rust
// PUT → Indexes properties and references
service.put("content", node).await?;

// DELETE → Removes from all indexes
service.delete("content", "article-1").await?;

// PUBLISH → Moves from draft to published indexes
service.publish("content", "/article").await?;

// UNPUBLISH → Moves from published to draft indexes
service.unpublish("content", "/article").await?;
```

No manual index management is required.

### Index Implementation in Custom Backends

When implementing a custom storage backend, you must implement these index repositories:

```rust
use raisin_storage::{PropertyIndexRepository, ReferenceIndexRepository};

pub struct MyPropertyIndex { /* your implementation */ }
pub struct MyReferenceIndex { /* your implementation */ }

impl PropertyIndexRepository for MyPropertyIndex {
    async fn index_properties(/* ... */) -> Result<()> { /* ... */ }
    async fn find_nodes_with_property(/* ... */) -> Result<Vec<String>> { /* ... */ }
    // ... other methods
}

impl ReferenceIndexRepository for MyReferenceIndex {
    async fn index_references(/* ... */) -> Result<()> { /* ... */ }
    async fn find_nodes_referencing(/* ... */) -> Result<Vec<String>> { /* ... */ }
    async fn find_outgoing_references(/* ... */) -> Result<HashMap<String, Vec<String>>> { /* ... */ }
    // ... other methods
}
```

See [Storage Traits API](../api/storage-traits.md) for complete interface documentation.

## Performance Comparison

| Backend | Read | Write | Memory | Persistence |
|---------|------|-------|--------|-------------|
| RocksDB | Fast | Very Fast | Low | Yes |
| InMemory | Very Fast | Very Fast | High | No |
| MongoDB* | Medium | Medium | Medium | Yes |
| PostgreSQL* | Medium | Medium | Low | Yes |

*Planned, not yet implemented

## Configuration

### RocksDB Options

```rust
use raisin_rocksdb::config::RocksDBConfig;
use raisin_rocksdb::open_db_with_config;

let config = RocksDBConfig::development().with_path("./data");
let db = open_db_with_config(&config)?;
```

### Multi-Tenancy

All backends support multi-tenancy through the `ScopedStorage` wrapper:

```rust
use raisin_storage::StorageExt;
use raisin_context::TenantContext;
use std::sync::Arc;

let storage = Arc::new(storage);
let ctx = TenantContext::new("tenant-123", "production");
let scoped = storage.scoped(ctx);
```

## Migration Between Backends

```rust
async fn migrate<S1, S2>(
    source: &S1,
    target: &S2,
    workspace: &str,
) -> Result<()>
where
    S1: Storage,
    S2: Storage,
{
    let nodes = source.nodes().list_all(workspace).await?;

    for node in nodes {
        target.nodes().put(workspace, node).await?;
    }

    Ok(())
}
```
