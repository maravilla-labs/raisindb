# Custom Storage Backends

This guide shows you how to implement a custom storage backend for RaisinDB.

## Storage Trait

All storage backends implement the `Storage` trait defined in `raisin-storage`. The trait has many associated types for all repository kinds:

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
    fn workspaces(&self) -> &Self::Workspaces;
    fn begin(&self) -> impl Future<Output = Result<Self::Tx>> + Send;
    fn event_bus(&self) -> Arc<dyn EventBus>;
    // ... accessors for all other repository types
}
```

> **Note**: Implementing a full storage backend requires implementing all 20+ repository types. For most use cases, it is easier to use the provided `RocksDBStorage` or the in-memory storage backend. A custom backend is typically only needed for specialized persistence requirements.

## Example: Custom Backend

Let's implement a simple file-based storage backend:

```rust
use raisin_storage::{Storage, NodeRepository, /* ... */};
use std::path::PathBuf;
use std::sync::Arc;

pub struct FileStorage {
    base_path: PathBuf,
    nodes: FileNodeRepo,
    node_types: FileNodeTypeRepo,
    workspaces: FileWorkspaceRepo,
}

impl FileStorage {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let base_path = path.into();
        std::fs::create_dir_all(&base_path)?;

        Ok(Self {
            base_path: base_path.clone(),
            nodes: FileNodeRepo::new(base_path.join("nodes")),
            node_types: FileNodeTypeRepo::new(base_path.join("node_types")),
            workspaces: FileWorkspaceRepo::new(base_path.join("workspaces")),
        })
    }
}

impl Storage for FileStorage {
    type Tx = FileTx;
    type Nodes = FileNodeRepo;
    type NodeTypes = FileNodeTypeRepo;
    type Workspaces = FileWorkspaceRepo;

    fn nodes(&self) -> &Self::Nodes {
        &self.nodes
    }

    fn node_types(&self) -> &Self::NodeTypes {
        &self.node_types
    }

    fn workspaces(&self) -> &Self::Workspaces {
        &self.workspaces
    }

    async fn begin(&self) -> Result<Self::Tx> {
        Ok(FileTx)
    }
}
```

## Implementing NodeRepository

```rust
pub struct FileNodeRepo {
    path: PathBuf,
}

impl FileNodeRepo {
    fn new(path: PathBuf) -> Self {
        std::fs::create_dir_all(&path).ok();
        Self { path }
    }

    fn node_path(&self, workspace: &str, id: &str) -> PathBuf {
        self.path.join(workspace).join(format!("{}.json", id))
    }
}

impl NodeRepository for FileNodeRepo {
    async fn get(&self, workspace: &str, id: &str) -> Result<Option<Node>> {
        let path = self.node_path(workspace, id);

        if !path.exists() {
            return Ok(None);
        }

        let data = tokio::fs::read_to_string(&path).await?;
        let node: Node = serde_json::from_str(&data)?;

        Ok(Some(node))
    }

    async fn put(&self, workspace: &str, node: Node) -> Result<()> {
        let path = self.node_path(workspace, &node.id);

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let data = serde_json::to_string_pretty(&node)?;
        tokio::fs::write(&path, data).await?;

        Ok(())
    }

    async fn delete(&self, workspace: &str, id: &str) -> Result<bool> {
        let path = self.node_path(workspace, id);

        if !path.exists() {
            return Ok(false);
        }

        tokio::fs::remove_file(&path).await?;
        Ok(true)
    }

    async fn list_all(&self, workspace: &str) -> Result<Vec<Node>> {
        let ws_path = self.path.join(workspace);

        if !ws_path.exists() {
            return Ok(Vec::new());
        }

        let mut nodes = Vec::new();
        let mut entries = tokio::fs::read_dir(&ws_path).await?;

        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension() == Some("json".as_ref()) {
                let data = tokio::fs::read_to_string(entry.path()).await?;
                let node: Node = serde_json::from_str(&data)?;
                nodes.push(node);
            }
        }

        Ok(nodes)
    }

    // Implement other methods...
}
```

## Multi-Tenancy Support

Add tenant prefixing to your custom backend:

```rust
impl FileNodeRepo {
    fn node_path(&self, workspace: &str, id: &str, ctx: Option<&TenantContext>) -> PathBuf {
        let base = if let Some(context) = ctx {
            self.path
                .join(context.tenant_id())
                .join(context.deployment())
        } else {
            self.path.clone()
        };

        base.join(workspace).join(format!("{}.json", id))
    }
}
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_custom_storage() {
        let dir = tempdir().unwrap();
        let storage = FileStorage::open(dir.path()).unwrap();

        let node = Node {
            id: "test-1".to_string(),
            name: "test".to_string(),
            /* ... */
        };

        storage.nodes().put("default", node.clone()).await.unwrap();

        let retrieved = storage.nodes()
            .get("default", "test-1")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retrieved.id, node.id);
    }
}
```

## Performance Tips

1. **Batch Operations**: Implement efficient batch reads/writes
2. **Caching**: Cache frequently accessed data
3. **Indexing**: Build indexes for common queries
4. **Connection Pooling**: Reuse connections for network-based backends

See the `raisin-rocksdb` crate (`crates/raisin-rocksdb/`) for a production-quality example of a full `Storage` implementation.
