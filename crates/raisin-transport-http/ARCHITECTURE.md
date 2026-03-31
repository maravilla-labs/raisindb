# raisin-transport-http Architecture

Internal architecture documentation for the HTTP transport layer.

## Overview

The transport layer follows a layered architecture with clear separation between:

1. **Routing** - URL pattern matching and handler dispatch
2. **Middleware** - Cross-cutting concerns (auth, parsing, CORS)
3. **Handlers** - Business logic for each endpoint
4. **State** - Shared application state and service access

## Request Flow

```
┌──────────────────────────────────────────────────────────────────────┐
│                        Incoming HTTP Request                          │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                          CORS Layer                                   │
│  - Per-origin configuration from AppState                            │
│  - Handles OPTIONS preflight requests                                │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                    ensure_tenant_middleware                           │
│  - Extract tenant_id, deployment_key from headers                    │
│  - Lazy-initialize NodeTypes for tenant if needed                    │
│  - Store TenantInfo in request extensions                            │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                 Authentication Middleware (RocksDB)                   │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │ require_auth_middleware      │ optional_auth_middleware         │ │
│  │ - Must have valid JWT        │ - Proceeds without auth          │ │
│  │ - Admin or User tokens       │ - Resolves auth if present       │ │
│  │ - Impersonation support      │ - Anonymous user fallback        │ │
│  └─────────────────────────────────────────────────────────────────┘ │
│                                                                       │
│  Token Validation Flow:                                               │
│  1. Try validate_token() -> AdminClaims                              │
│  2. If fails, try validate_user_token() -> AuthClaims               │
│  3. Create AuthContext with resolved permissions                     │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                   raisin_parsing_middleware                           │
│  (Applied to /api/repository/* routes only)                          │
│                                                                       │
│  Input:  /api/repository/myrepo/main/head/content/blog/post@file    │
│                                                                       │
│  Output (RaisinContext):                                             │
│  - repo_name: "myrepo"                                               │
│  - branch_name: "main"                                               │
│  - workspace_name: "content"                                         │
│  - cleaned_path: "/blog/post"                                        │
│  - property_path: Some("file")                                       │
│  - is_command: false                                                  │
│  - is_version: false                                                  │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                        Route Handler                                  │
│  - Extract path params, query params, body                           │
│  - Get RaisinContext from extensions                                 │
│  - Call NodeService with proper scoping                              │
│  - Return structured response or ApiError                            │
└──────────────────────────────────────────────────────────────────────┘
```

## AppState Structure

```rust
pub struct AppState {
    // Core storage
    storage: Arc<Store>,              // RocksDB or InMemory
    connection: Arc<RaisinConnection>,
    ws_svc: Arc<WorkspaceService>,

    // Binary storage
    bin: Arc<Bin>,                    // Filesystem or S3

    // Audit
    audit: Arc<AuditRepo>,
    audit_adapter: Arc<RepoAuditAdapter>,

    // Upload handling
    upload_processors: Arc<UploadProcessorRegistry>,

    // Config
    anonymous_enabled: bool,

    // RocksDB-specific (optional)
    indexing_engine: Option<Arc<TantivyIndexingEngine>>,
    tantivy_management: Option<Arc<TantivyManagement>>,
    embedding_storage: Option<Arc<RocksDBEmbeddingStorage>>,
    hnsw_engine: Option<Arc<HnswIndexingEngine>>,
    auth_service: Option<Arc<AuthService>>,
}
```

## Handler Organization

Handlers are organized by domain:

```
handlers/
├── mod.rs              # Module declarations
├── repo.rs             # Repository CRUD operations
├── query.rs            # JSON and DSL queries
├── nodes.rs            # Node-specific operations
├── auth.rs             # Admin authentication
├── identity_auth.rs    # User authentication (OIDC, magic link)
├── sql.rs              # SQL query execution
├── functions.rs        # Serverless function invocation
├── webhooks.rs         # HTTP webhook triggers
├── packages.rs         # Package management
├── uploads.rs          # Resumable file uploads
├── workspaces.rs       # Workspace management
├── branches.rs         # Branch management
├── tags.rs             # Tag management
├── revisions.rs        # Revision history
├── node_types.rs       # NodeType schema management
├── archetypes.rs       # Archetype management
├── element_types.rs    # ElementType management
├── repositories.rs     # Repository management
├── translations.rs     # Translation management
├── audit.rs            # Audit log queries
├── registry.rs         # Tenant/deployment registry
├── embeddings.rs       # Embedding configuration
├── ai.rs               # AI configuration
├── processing_rules.rs # Processing rule management
├── admin_users.rs      # Admin user management
├── identity_users.rs   # Identity user management
├── profile.rs          # User profile/API keys
├── hybrid_search.rs    # Hybrid search (text + vector)
├── workspace_access.rs # Workspace access control
├── replication.rs      # Replication sync endpoints
├── system_updates.rs   # System update endpoints
└── management/
    ├── mod.rs          # Management module declarations
    ├── global.rs       # Global RocksDB operations
    ├── tenant.rs       # Tenant-wide operations
    └── database.rs     # Repository-specific index operations
```

## URL Path Parsing

The `raisin_parsing_middleware` handles sophisticated path parsing:

### URL Format

```
/api/repository/{repo}/{branch}/{head|rev/{n}}/{workspace}/{path}[@property][/raisin:cmd/{cmd}]
```

### Path Components

| Component | Required | Example |
|-----------|----------|---------|
| `repo` | Yes | `myrepo` |
| `branch` | Yes | `main`, `feature-x` |
| `head` or `rev/{n}` | Yes | `head`, `rev/12345` |
| `workspace` | Yes | `content`, `assets` |
| `path` | No | `/blog/my-post` |

### Path Modifiers

**Property Access (`@property`)**
```
/content/blog/post@properties.title   -> property_path = "properties.title"
/assets/image@file                    -> property_path = "file"
```

**Version Access (`raisin:version`)**
```
/content/blog/post/raisin:version     -> is_version = true, version_id = None
/content/blog/post/raisin:version/5   -> is_version = true, version_id = Some(5)
```

**Command Execution (`raisin:cmd`)**
```
/content/blog/post/raisin:cmd/download  -> is_command = true, command_name = "download"
/assets/image/raisin:cmd/relations      -> is_command = true, command_name = "relations"
```

## Authentication Flow

### Admin Token (AdminClaims)

```
Authorization: Bearer <admin_jwt>
X-Raisin-Impersonate: <user_id>  (optional)

1. validate_token() -> AdminClaims
2. If impersonation requested:
   - Check can_impersonate flag
   - Resolve target user permissions
   - Create impersonated AuthContext
3. Else:
   - Create system AuthContext (bypasses RLS)
```

### User Token (AuthClaims)

```
Authorization: Bearer <user_jwt>

1. validate_user_token() -> AuthClaims
2. Resolve permissions via PermissionService
3. Create user AuthContext with resolved permissions
```

### Anonymous Access

```
No Authorization header

1. Check anonymous_enabled config (repo -> tenant -> global)
2. If enabled:
   - Resolve anonymous user permissions
   - Create user AuthContext for anonymous user
3. If disabled:
   - Create deny-all AuthContext
```

## Error Handling

### ApiError Structure

```rust
pub struct ApiError {
    code: String,       // Machine-readable: "NODE_NOT_FOUND"
    message: String,    // Human-friendly message
    details: Option<String>,  // Technical details
    field: Option<String>,    // Field name for validation
    timestamp: String,  // ISO 8601 timestamp
    status: StatusCode, // HTTP status
}
```

### Error Conversion

```rust
impl From<raisin_error::Error> for ApiError {
    fn from(err: raisin_error::Error) -> Self {
        match err {
            Error::NotFound(msg) => ApiError::node_not_found(...),
            Error::Validation(msg) => ApiError::validation_failed(...),
            Error::Conflict(msg) => ApiError::node_already_exists(...),
            Error::Unauthorized(msg) => ApiError::unauthorized(...),
            Error::Forbidden(msg) => ApiError::forbidden(...),
            // ...
        }
    }
}
```

## Feature Flags

```toml
[features]
default = ["fs", "storage-rocksdb"]

# Binary storage backends
fs = []                    # Filesystem storage
s3 = ["raisin-binary/s3"]  # S3/R2 storage

# Database backends
store-memory = ["dep:raisin-storage-memory"]
storage-rocksdb = [
    "dep:raisin-rocksdb",
    "dep:raisin-indexer",
    "dep:raisin-hnsw",
    "dep:raisin-sql-execution",
    "dep:rocksdb",
    "dep:raisin-replication",
    "dep:raisin-flow-runtime",
    "dep:raisin-auth"
]
```

### Conditional Compilation

Many handlers and middleware are only available with RocksDB:

```rust
#[cfg(feature = "storage-rocksdb")]
pub mod auth;

#[cfg(feature = "storage-rocksdb")]
pub async fn require_auth_middleware(...) { ... }
```

## Upload Processing

### Upload Processor Trait

```rust
pub trait UploadProcessor: Send + Sync {
    fn node_type(&self) -> &str;

    fn process(&self,
        filename: &str,
        content_type: &str,
        data: &[u8]
    ) -> Result<ProcessedUpload, ApiError>;
}
```

### ProcessedUpload Result

```rust
pub struct ProcessedUpload {
    node_id: Option<String>,      // Override generated ID
    node_name: Option<String>,    // Override filename-derived name
    properties: HashMap<String, PropertyValue>,  // Extra properties
    resource_property: String,    // Target property (default: "file")
    storage_format: StorageFormat, // Resource or Object
}
```

### Built-in Processors

- **Package Processor** - Extracts manifest.yaml from .rap files
- **Default Processor** - Standard file upload handling

## Response Envelope

### Paginated Response

```json
{
  "items": [...],
  "page": {
    "total": 100,
    "limit": 20,
    "offset": 0,
    "nextOffset": 20
  }
}
```

### Cursor-based Pagination

```json
{
  "items": [...],
  "cursor": "base64-encoded-cursor",
  "hasMore": true
}
```

## CORS Configuration

### Unified CORS (`unified_cors_middleware`)

All routes pass through `unified_cors_middleware` which resolves origins hierarchically:

1. **Repo-level** — `RepoAuthConfig.cors_allowed_origins` (highest priority)
2. **All-repos aggregation** — when no repo in URL (e.g. `/api/uploads`), origins from every repo are aggregated
3. **Tenant-level** — `TenantAuthConfig.cors_allowed_origins`
4. **Global** — TOML `cors_allowed_origins` (fallback)

Results are cached in `AppState.cors_cache` (`TtlCache<Vec<String>>`, 60s TTL) keyed by `{tenant}/{repo}` or `{tenant}/__all__`.

### Per-Repository CORS (auth routes)

Applied via `repo_auth_cors_middleware` for `/auth/{repo}/*` routes:

1. Extract repo ID from path
2. Load RepoAuthConfig from system workspace
3. Check `cors_allowed_origins` property
4. Add appropriate headers to response
