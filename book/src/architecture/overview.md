# Architecture Overview

RaisinDB is built with a layered architecture that separates concerns and allows for flexibility.

## High-Level Architecture

```text
┌─────────────────────────────────────────────────┐
│  Application / HTTP Server                      │
│  (Your code or raisin-server)                   │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  Service Layer (raisin-core)                    │
│  - NodeService                                  │
│  - WorkspaceService                             │
│  - Validation, Audit, Versioning                │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  Storage Abstraction (raisin-storage)           │
│  - Storage trait                                │
│  - Repository traits                            │
│  - Multi-tenancy support                        │
└─────────────────────────────────────────────────┘
                    ↓
        ┌───────────┴───────────┐
        ↓                       ↓
┌──────────────────┐    ┌──────────────────┐
│  RocksDB         │    │  InMemory        │
│  (Default)       │    │  (Testing)       │
└──────────────────┘    └──────────────────┘
```

## Core Components

### Models (`raisin-models`)
- `Node`: Content nodes with properties
- `NodeType`: Schema definitions
- `Workspace`: Organizational units
- Property types and validation rules

### Services (`raisin-core`)
- **NodeService**: CRUD operations for nodes
- **WorkspaceService**: Workspace management
- **Validation**: Schema validation
- **Audit**: Change tracking

### Storage (`raisin-storage`)
- **Trait-based abstraction**: Allows multiple backends
- **Repository pattern**: Separate concerns
- **Transaction support**: ACID operations
- **Multi-tenancy**: Scoped storage

### Transport
- **HTTP** (`raisin-transport-http`): Axum-based RESTful API
- **WebSocket** (`raisin-transport-ws`): Real-time event streaming
- **PGWire** (`raisin-transport-pgwire`): PostgreSQL wire protocol (connect via `psql`)
- **Middleware**: Authentication, tenant resolution

## Data Flow

### Creating a Node

```text
1. HTTP Request
   ↓
2. Route Handler
   ↓
3. Tenant Resolution (if multi-tenant)
   ↓
4. NodeService::add_node()
   ↓
5. Validation (schema, constraints)
   ↓
6. Storage::nodes().put()
   ↓
7. RocksDB Write
   ↓
8. Audit Log (optional)
   ↓
9. HTTP Response
```

## Design Principles

1. **Separation of Concerns**: Clear layer boundaries
2. **Trait-based Abstractions**: Flexible, testable
3. **Type Safety**: Leverage Rust's type system
4. **Performance**: Zero-cost abstractions where possible
5. **Extensibility**: Easy to add new backends

## Multi-Tenancy Support

RaisinDB supports multi-tenancy through:

- **Scoped Services**: Automatically apply tenant context
- **Storage Prefixing**: Logical isolation at storage level
- **Pluggable Resolution**: Custom tenant extraction
- **Rate Limiting**: Per-tenant limits

See [Multi-Tenancy](./multi-tenancy.md) for details.

## Storage Backends

Currently supported:

- **RocksDB**: Production-ready, high-performance
- **InMemory**: Fast, for testing

Planned:
- MongoDB
- PostgreSQL

See [Storage Backends](./storage-backends.md) for details.
