# raisin-core

Core business logic and service layer for RaisinDB.

## Overview

This crate sits between the storage layer (`raisin-storage`) and transport layers (`raisin-transport-http`, `raisin-transport-ws`). It provides:

- **RaisinConnection** - MongoDB-inspired fluent API for server-side operations
- **NodeService** - CRUD, tree management, publishing, and versioning for nodes
- **WorkspaceService** - Workspace management
- **NodeTypeResolver** - Node type definition loading and resolution
- **NodeValidator** - Schema validation against NodeType definitions
- **TranslationResolver** - Locale fallback chains and translation merging
- **TranslationService** - Translation management operations
- **BlockTranslationService** - Block-level translations with UUID tracking
- **PermissionService** - Permission resolution from users/groups/roles
- **ReferenceResolver** - Reference property resolution
- **Transaction** - Atomic multi-node operations

## Architecture

```
RaisinConnection<S: Storage>     (server connection with storage backend)
  ↓
TenantScope<'c, S>               (tenant isolation - always required)
  ↓
Repository<S>                    (repository/database handle)
  ↓
Workspace<S>                     (workspace within repository)
  ↓
NodeServiceBuilder               (fluent API with .branch() and .revision())
  ↓
NodeService                      (CRUD operations on nodes)
```

## Usage

```rust
use raisin_core::RaisinConnection;
use raisin_storage_memory::InMemoryStorage;
use std::sync::Arc;

// Server initialization
let storage = Arc::new(InMemoryStorage::default());
let connection = RaisinConnection::with_storage(storage);

// Tenant scoping (always required)
let tenant = connection.tenant("acme-corp");

// Repository access
let repo = tenant.repository("website");

// Workspace operations
let workspace = repo.workspace("main");
let nodes = workspace.nodes();

// CRUD operations
let node = nodes.get("node-id").await?;

// Branch-specific operations
let develop_nodes = workspace.nodes().branch("develop");

// Time-travel queries
let historical = workspace.nodes().revision(42);
```

### Transactions

```rust
let mut tx = workspace.nodes().transaction();
tx.create(node1);
tx.update(node2_id, props);
tx.delete(node3_id);
tx.commit("Bulk update", "user-123").await?;
```

### Translation Resolution

```rust
use raisin_core::TranslationResolver;

let resolver = TranslationResolver::new(repository, config);
let translated = resolver.resolve_node(
    tenant_id, repo_id, branch, workspace,
    node, &locale, &revision
).await?;
```

## Components

### Services (`src/services/`)

| Service | Description |
|---------|-------------|
| `node_service/` | Node CRUD, tree ops, publishing, versioning |
| `workspace_service.rs` | Workspace management |
| `node_type_resolver.rs` | NodeType loading and resolution |
| `node_validation.rs` | Schema validation |
| `translation_resolver.rs` | Locale fallback and translation merging |
| `translation_service.rs` | Translation CRUD operations |
| `block_translation_service.rs` | Block-level translation management |
| `permission_service.rs` | User/group/role permission resolution |
| `ttl_cache.rs` | Generic DashMap-based TTL cache, reusable across services |
| `permission_cache.rs` | Permission-specific cache wrapper around TtlCache |
| `reference_resolver.rs` | Reference property resolution |
| `rls_filter.rs` | Row-level security filtering |
| `transaction.rs` | Atomic multi-node operations |
| `indexing_policy.rs` | Index type selection for properties |

### Initialization (`src/`)

| Module | Description |
|--------|-------------|
| `init.rs` | Tenant/global NodeType initialization |
| `nodetype_init.rs` | Built-in NodeType loading from YAML |
| `workspace_init.rs` | Built-in workspace initialization |
| `workspace_structure_init.rs` | Initial workspace structure creation |
| `package_init.rs` | Built-in package loading |
| `system_updates/` | Breaking change detection and pending updates |

### Other

| Module | Description |
|--------|-------------|
| `connection.rs` | `RaisinConnection` fluent API |
| `traits.rs` | `Audit` trait for pluggable audit logging |
| `audit_adapter.rs` | Repository-aware audit adapter |
| `replication/` | Peer configuration and sync coordination |
| `utils.rs` | `sanitize_name`, asset URL signing |

## Dependencies

- `raisin-storage` - Storage trait abstractions
- `raisin-context` - Multi-tenancy and repository context
- `raisin-models` - Data models (Node, NodeType, Workspace, etc.)
- `raisin-validation` - Property validation
- `raisin-indexer` - Indexing abstractions
- `raisin-hlc` - Hybrid Logical Clock for versioning
- `raisin-error` - Error types

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
