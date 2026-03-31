# Architecture

## Design Philosophy

raisin-core implements a **repository-first architecture** with a MongoDB-inspired fluent API. All operations are scoped through a tenant → repository → workspace → branch hierarchy, ensuring multi-tenant isolation at every level.

## Core Abstractions

### RaisinConnection

The main entry point for server-side operations:

```
┌─────────────────────────────────────────────────────────────┐
│                    RaisinConnection<S>                       │
│                                                              │
│  storage: Arc<S>           (storage backend)                │
│  config: ServerConfig      (default branch, auto-create)    │
│                                                              │
│  Methods:                                                    │
│  - tenant(id) → TenantScope                                 │
│  - repository_management() → RepositoryManagement           │
└─────────────────────────────────────────────────────────────┘
```

### Scoping Hierarchy

```
RaisinConnection
      │
      ├── tenant("acme-corp")
      │         │
      │         └── TenantScope
      │                   │
      │                   ├── repository("website")
      │                   │         │
      │                   │         └── Repository
      │                   │                   │
      │                   │                   └── workspace("main")
      │                   │                             │
      │                   │                             └── Workspace
      │                   │                                     │
      │                   │                                     └── nodes()
      │                   │                                           │
      │                   │                                           └── NodeServiceBuilder
      │                   │                                                 │
      │                   │                                                 ├── .branch("develop")
      │                   │                                                 ├── .revision(42)
      │                   │                                                 └── CRUD operations
      │                   │
      │                   └── list_repositories()
      │
      └── repository_management()
                  │
                  └── RepositoryManagement (admin ops)
```

## NodeService Architecture

NodeService is split into focused submodules:

```
┌─────────────────────────────────────────────────────────────┐
│                       NodeService<S>                         │
│                                                              │
│  storage: Arc<S>                                            │
│  tenant_id, repo_id, branch, workspace                      │
│  audit: Option<Arc<dyn Audit>>                              │
│  auth_context: Option<AuthContext>                          │
│                                                              │
├─────────────────────────────────────────────────────────────┤
│  Submodules:                                                │
│                                                              │
│  node_creation_helpers   - ID generation, path building     │
│  legacy_creation         - put/create with validation       │
│  property_operations     - get/set property by path         │
│  tree_operations         - move, rename, copy, reorder      │
│  copy_publish            - publish/unpublish workflows      │
│  versioning              - version history, compare         │
│  branch_tag_operations   - branch/tag management            │
│  relationship_operations - relation queries                 │
│  transactional           - transaction support              │
└─────────────────────────────────────────────────────────────┘
```

### Node Validation Flow

```
put(node)
    │
    ├── Load NodeType definition
    │         │
    │         └── NodeTypeResolver.resolve_type()
    │                   │
    │                   └── Resolve inheritance chain (extends, mixins)
    │
    ├── Validate properties
    │         │
    │         └── NodeValidator.validate()
    │                   │
    │                   ├── Check required properties
    │                   ├── Validate property types
    │                   ├── Apply constraints
    │                   └── Validate nested structures
    │
    ├── Apply RLS checks (if auth_context present)
    │         │
    │         └── rls_filter::can_perform()
    │
    ├── Persist to storage
    │
    └── Write audit log (if audit configured)
```

## Translation Resolution

Translation resolution applies locale fallback chains:

```
resolve_node(node, locale="fr-CA")
    │
    ├── Get fallback chain from RepositoryConfig
    │         │
    │         └── ["fr-CA", "fr", "en"]
    │
    └── For each locale in chain:
              │
              ├── Fetch LocaleOverlay from TranslationRepository
              │
              ├── If Hidden → return None (node hidden in this locale)
              │
              └── If Properties → merge overlay into node
                        │
                        ├── Scalar properties: direct override
                        │
                        └── Composite (blocks): UUID-based merging
                                  │
                                  ├── Match blocks by UUID
                                  ├── Preserve block order from base
                                  └── Apply translated content
```

### Block Translation Model

```
Base Node (en)                    Translation (fr)
─────────────                     ────────────────
blocks: [                         blocks: {
  { uuid: "a1", text: "Hello" },    "a1": { text: "Bonjour" },
  { uuid: "b2", text: "World" }     "b2": { text: "Monde" }
]                                 }

Result (fr):
────────────
blocks: [
  { uuid: "a1", text: "Bonjour" },
  { uuid: "b2", text: "Monde" }
]
```

## Permission Resolution

Permission resolution traverses user → groups → roles:

```
resolve_for_user_id(user_id)
    │
    ├── Load User node from access_control workspace
    │
    ├── Collect direct roles from user.roles[]
    │
    ├── Load user's groups
    │         │
    │         └── For each group: collect group.roles[]
    │
    ├── Resolve role inheritance (recursive)
    │         │
    │         └── For each role: follow role.inherits[]
    │
    └── Flatten all permissions into ResolvedPermissions
              │
              ├── permissions: Vec<Permission>
              ├── conditions: HashMap<RoleCondition>
              ├── is_admin: bool
              └── groups: Vec<String>
```

### TtlCache

Generic, DashMap-based TTL cache used across services:

```
┌─────────────────────────────────────────────────────────┐
│                 TtlCache<V: Clone + Send + Sync>         │
│                                                          │
│  ┌────────────────────────────────────────────────┐     │
│  │         DashMap<String, CacheEntry<V>>          │     │
│  │                                                 │     │
│  │  Key: String                                   │     │
│  │  Value: CacheEntry { value: V, cached_at }     │     │
│  └────────────────────────────────────────────────┘     │
│                                                          │
│  API: get, put, get_or_compute, invalidate,             │
│       invalidate_many, invalidate_all, cleanup_expired  │
│  TTL: configurable (default 5 minutes)                  │
│  Lock-free: DashMap for concurrent async access          │
└─────────────────────────────────────────────────────────┘
```

### Permission Cache

Delegates to `TtlCache<ResolvedPermissions>`:

```
┌─────────────────────────────────────────────────────────┐
│                   PermissionCache                        │
│                                                          │
│  inner: TtlCache<ResolvedPermissions>                   │
│                                                          │
│  Key: user_id                                           │
│  Value: ResolvedPermissions                             │
│  TTL: configurable (default 5 minutes)                  │
└─────────────────────────────────────────────────────────┘
```

## Transaction Model

Transactions accumulate operations and commit atomically:

```rust
let mut tx = workspace.nodes().transaction();
tx.create(node1);                    // Queue create
tx.update("node-2", props);          // Queue update
tx.delete("node-3");                 // Queue delete
tx.commit("message", "actor").await; // Atomic commit
```

```
Transaction
    │
    ├── operations: Vec<TxOperation>
    │         │
    │         ├── Create(Node)
    │         ├── Update { id, properties }
    │         └── Delete { id }
    │
    └── commit()
              │
              ├── Begin storage transaction
              ├── Execute all operations
              ├── Generate single HLC revision
              └── Commit or rollback
```

## RLS (Row-Level Security) Filter

RLS checks are applied at multiple points:

```
┌─────────────────────────────────────────────────────────┐
│                     RLS Filter                           │
│                                                          │
│  Entry Points:                                          │
│  - can_perform(auth, node, operation)                   │
│  - can_create_at_path(auth, parent_path, node_type)     │
│  - filter_node(auth, node) → Option<Node>               │
│  - filter_nodes(auth, nodes) → Vec<Node>                │
│                                                          │
│  Checks:                                                │
│  1. Is user admin? → allow all                          │
│  2. Check explicit permissions for operation            │
│  3. Apply path-based conditions                         │
│  4. Apply owner-based conditions                        │
│  5. Apply group-based conditions                        │
└─────────────────────────────────────────────────────────┘
```

## Initialization Flow

On server startup or tenant creation:

```
init_tenant_nodetypes(tenant_id, repo_id)
    │
    ├── Load global NodeTypes from embedded YAML
    │         │
    │         └── include_dir!("src/nodetypes/")
    │
    ├── Calculate version hash for each NodeType
    │
    ├── Check existing NodeTypes in storage
    │
    └── Upsert changed NodeTypes

init_repository_workspaces(tenant_id, repo_id)
    │
    ├── Load workspace definitions from embedded YAML
    │
    └── Create workspaces if not exist

create_workspace_initial_structure(workspace)
    │
    └── Create initial node tree structure
```

## System Updates

Breaking change detection for schema migrations:

```
check_pending_updates(storage)
    │
    ├── Load current NodeType hashes from storage
    │
    ├── Compare with embedded NodeType hashes
    │
    └── Return list of breaking changes
              │
              ├── RemovedProperty
              ├── TypeChanged
              ├── RequiredAdded
              └── ValidationChanged
```

## Multi-Tenancy

Every operation flows through tenant scoping:

```
┌─────────────────────────────────────────────────────────┐
│                   Storage Keys                           │
│                                                          │
│  Pattern: /{tenant_id}/repo/{repo_id}/...               │
│                                                          │
│  Examples:                                              │
│  /acme-corp/repo/website/branch/main/nodes/abc123       │
│  /acme-corp/repo/website/workspace/main/config          │
│  /default/repo/my-app/nodetypes/Article                 │
└─────────────────────────────────────────────────────────┘
```

For embedded/single-tenant deployments, use `"default"` as tenant_id.
