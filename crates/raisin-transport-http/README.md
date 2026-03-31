# raisin-transport-http

Axum-based HTTP REST API for RaisinDB with comprehensive endpoint coverage.

## Overview

This crate provides the complete HTTP transport layer for RaisinDB, enabling REST API access to all core functionality including content management, authentication, search, and administrative operations.

- **RESTful API Design** - Path-based repository access with HEAD/revision support
- **Multi-Tenant Support** - Tenant isolation via headers and middleware
- **Dual Authentication** - Admin JWT tokens and identity-based user tokens
- **Flexible Storage** - Pluggable backends (RocksDB, in-memory) with feature flags
- **Resumable Uploads** - Chunked upload support for large files (up to 50GB)
- **Real-time Search** - Full-text (Tantivy) and vector (HNSW) search integration

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      HTTP Request                                │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                      MIDDLEWARE STACK                            │
│  ┌────────────────┐  ┌────────────────┐  ┌──────────────────┐   │
│  │  CORS Layer    │  │ Tenant Ensure  │  │   Auth Layer     │   │
│  │ (per-origin)   │  │  (NodeType     │  │ (require/optional│   │
│  │                │  │   init)        │  │  admin/user)     │   │
│  └────────────────┘  └────────────────┘  └──────────────────┘   │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │              Raisin Parsing Middleware                      │ │
│  │  - Extract repo/branch/workspace from path                  │ │
│  │  - Parse commands (raisin:cmd/*), versions (raisin:version) │ │
│  │  - Extract property paths (@property notation)              │ │
│  └────────────────────────────────────────────────────────────┘ │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                       ROUTE HANDLERS                             │
├──────────────────┬──────────────────┬───────────────────────────┤
│   Repository     │   Management     │       System              │
│   /api/repository│   /api/management│   /api/admin/management   │
│   - CRUD nodes   │   - NodeTypes    │   - RocksDB ops           │
│   - Queries      │   - Branches     │   - Index rebuild         │
│   - Commands     │   - Tags         │   - Tenant cleanup        │
├──────────────────┼──────────────────┼───────────────────────────┤
│   Auth           │   Functions      │       Search              │
│   /auth/*        │   /api/functions │   /api/sql, /api/search   │
│   - OIDC         │   - Invoke       │   - SQL queries           │
│   - Magic Link   │   - Executions   │   - Hybrid search         │
│   - Sessions     │   - Flows        │   - Full-text search      │
├──────────────────┴──────────────────┴───────────────────────────┤
│   Packages & Webhooks                                            │
│   /api/packages, /api/webhooks, /api/triggers                   │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                        AppState                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   Storage   │  │  Binary     │  │  Optional (RocksDB)     │  │
│  │  (RocksDB/  │  │  Storage    │  │  - Tantivy indexing     │  │
│  │   Memory)   │  │  (FS/S3)    │  │  - HNSW embeddings      │  │
│  │             │  │             │  │  - Auth service         │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## Usage

### Basic Router Setup

```rust
use raisin_transport_http::router_with_bin_and_audit;
use std::sync::Arc;

// Create router with all dependencies
let (router, state) = router_with_bin_and_audit(
    storage,
    workspace_service,
    binary_storage,
    audit_repo,
    audit_adapter,
    anonymous_enabled,
    &cors_origins,
    // RocksDB-specific optional components
    indexing_engine,
    tantivy_management,
    embedding_storage,
    embedding_job_store,
    hnsw_engine,
    hnsw_management,
    rocksdb_storage,
    auth_service,
);

// Run with Axum
let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
axum::serve(listener, router).await?;
```

### URL Patterns

```
# Repository operations (HEAD - mutable)
GET/POST/PUT/DELETE /api/repository/{repo}/{branch}/head/{workspace}/{path}

# Repository operations (revision - read-only)
GET /api/repository/{repo}/{branch}/rev/{revision}/{workspace}/{path}

# Commands via path
GET /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/download
POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/publish

# Property access
GET /api/repository/{repo}/{branch}/head/{ws}/{path}@properties.file

# SQL queries
POST /api/sql/{repo}
POST /api/sql/{repo}/{branch}

# Authentication
POST /auth/login
POST /auth/{repo}/register
GET /auth/providers
```

## API Categories

### Repository API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/repository/{repo}/{branch}/head/{ws}/` | GET | List root nodes |
| `/api/repository/{repo}/{branch}/head/{ws}/{path}` | GET | Get node by path |
| `/api/repository/{repo}/{branch}/head/{ws}/{path}` | POST | Create child node |
| `/api/repository/{repo}/{branch}/head/{ws}/{path}` | PUT | Update node |
| `/api/repository/{repo}/{branch}/head/{ws}/{path}` | DELETE | Delete node |
| `/api/repository/{repo}/{branch}/head/{ws}/query` | POST | JSON filter query |
| `/api/repository/{repo}/{branch}/head/{ws}/query/dsl` | POST | DSL query |

### Management API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/management/{repo}/{branch}/nodetypes` | GET/POST | List/create NodeTypes |
| `/api/management/{repo}/{branch}/archetypes` | GET/POST | List/create Archetypes |
| `/api/management/repositories/{tenant}/{repo}/branches` | GET/POST | Branch management |
| `/api/management/repositories/{tenant}/{repo}/tags` | GET/POST | Tag management |
| `/api/management/repositories/{tenant}/{repo}/revisions` | GET | Revision history |

### Authentication API (RocksDB)

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/auth/login` | POST | Email/password login |
| `/auth/register` | POST | User registration |
| `/auth/magic-link` | POST | Passwordless login |
| `/auth/oidc/{provider}` | GET | OIDC authorization |
| `/auth/{repo}/register` | POST | Repository-scoped registration |
| `/auth/sessions` | GET | List user sessions |

### Search API (RocksDB)

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/sql/{repo}` | POST | Execute SQL query |
| `/api/search/{repo}` | GET | Hybrid search (text + vector) |
| `/api/repository/{repo}/{branch}/fulltext/search` | POST | Full-text search |

## Modules

| Module | Description |
|--------|-------------|
| `routes` | Route definitions and endpoint registration |
| `state` | AppState and router construction |
| `middleware` | Parsing, auth, tenant, CORS middleware |
| `error` | Structured API error responses |
| `handlers/` | Request handlers organized by domain |
| `types` | Request/response DTOs |
| `upload_processors` | NodeType-specific upload handling |

### Handler Modules

| Handler | Description |
|---------|-------------|
| `repo` | Repository CRUD and commands |
| `query` | JSON and DSL queries |
| `auth` | Admin authentication |
| `identity_auth` | Identity-based authentication (OIDC, magic link) |
| `sql` | SQL query execution |
| `functions` | Serverless function invocation |
| `webhooks` | HTTP webhook triggers |
| `management/` | NodeType, Branch, Tag management |

## Features

```toml
[dependencies]
raisin-transport-http = { version = "0.1", features = ["storage-rocksdb"] }
```

| Feature | Description | Default |
|---------|-------------|---------|
| `fs` | Filesystem binary storage | Yes |
| `storage-rocksdb` | RocksDB backend with full features | Yes |
| `s3` | S3/R2 binary storage | No |
| `store-memory` | In-memory storage for testing | No |

## CORS

CORS origins are resolved hierarchically: **Repo > Tenant > Global**.

For routes without a repo in the URL (e.g. `/api/uploads`), origins are aggregated across all repos for the tenant. Results are cached with a 60-second TTL via `TtlCache<Vec<String>>` to avoid repeated storage queries.

## Error Handling

Structured error responses with machine-readable codes:

```json
{
  "code": "NODE_NOT_FOUND",
  "message": "Node not found at path: /blog/post",
  "details": "...",
  "timestamp": "2025-01-15T10:30:00Z"
}
```

Common error codes:
- `NODE_NOT_FOUND`, `BRANCH_NOT_FOUND`, `WORKSPACE_NOT_FOUND`
- `VALIDATION_FAILED`, `INVALID_NODE_TYPE`, `MISSING_REQUIRED_FIELD`
- `NODE_ALREADY_EXISTS`, `BRANCH_ALREADY_EXISTS`
- `READ_ONLY_REVISION`, `UNAUTHORIZED`, `FORBIDDEN`

## Documentation

- [REST API Reference](docs/rest-api.md) - Detailed endpoint documentation
- [ARCHITECTURE.md](ARCHITECTURE.md) - Internal architecture details

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
