---
sidebar_position: 3
---

# Architecture Overview

RaisinDB is built with a modular, scalable architecture designed for high performance and reliability.

## System Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   HTTP Client   │    │   Admin Console │    │   External API  │
└─────────────────┘    └─────────────────┘    └─────────────────┘
          │                       │                       │
          └───────────────────────┼───────────────────────┘
                                  │
                    ┌─────────────────┐
                    │  HTTP Transport │  (raisin-transport-http)
                    │   REST API      │
                    └─────────────────┘
                                  │
                    ┌─────────────────┐
                    │   Core Engine   │  (raisin-core)
                    │ Business Logic  │
                    └─────────────────┘
                                  │
                    ┌─────────────────┐
                    │  Storage Layer  │  (raisin-storage)
                    │   RocksDB       │
                    └─────────────────┘
```

## Core Components

### HTTP Transport Layer (`raisin-transport-http`)

The REST API layer that provides:
- **RESTful endpoints** for all operations
- **Request/response handling** with proper HTTP status codes
- **Input validation** and sanitization
- **Error handling** with meaningful error messages
- **Authentication & authorization** middleware
- **Multi-tenant routing** based on repository context

Key endpoints:
- `/api/workspaces/{repo}` - Workspace management
- `/api/nodes/{repo}/{workspace}` - Node CRUD operations
- `/api/branches/{repo}` - Branch and tag management
- `/api/query/{repo}/{workspace}` - Advanced querying

### Core Engine (`raisin-core`)

The business logic layer that handles:
- **NodeType management** and validation
- **Tree operations** (parent-child relationships)
- **Version control** logic (commits, branches, merging)
- **Transaction management** for atomic operations
- **Reference resolution** and integrity checking
- **Workspace isolation** and multi-tenancy

Services:
- `NodeService` - Core node operations
- `WorkspaceService` - Workspace management  
- `TransactionService` - ACID transaction handling
- `NodeTypeResolver` - Schema validation and resolution

### Storage Layer (`raisin-storage`)

The persistence layer built on RocksDB:
- **High-performance storage** with RocksDB backend
- **Atomic transactions** across multiple operations
- **Efficient indexing** for fast queries
- **Repository isolation** with separate RocksDB instances
- **Backup and recovery** capabilities
- **Job management** for background tasks

## Data Model

### Physical Storage

RaisinDB uses RocksDB's key-value storage with structured keys:

```
Repository: {repo_name}
├── nodes:{workspace}:{node_id} → Node data
├── tree:{workspace}:{parent_id}:{child_id} → Parent-child relationships
├── commits:{workspace}:{commit_id} → Commit metadata
├── branches:{branch_name} → Branch pointers
├── tags:{tag_name} → Tag pointers
└── nodetypes:{nodetype_name} → NodeType definitions
```

### Logical Organization

```
Repository
├── Workspaces (isolated environments)
│   ├── Nodes (data records)
│   │   ├── Properties (key-value data)
│   │   └── Relationships (parent-child links)
│   ├── Commits (version history)
│   └── Branches (parallel development)
├── NodeTypes (schema definitions)
└── Configuration (repository settings)
```

## Multi-tenancy

### Repository-level Isolation

Each repository is completely isolated:
- **Separate RocksDB instances** for data isolation
- **Independent NodeType registries** for schema flexibility
- **Isolated transaction contexts** for consistency
- **Per-repository configuration** for customization

### Workspace-level Isolation

Within repositories, workspaces provide:
- **Data isolation** - changes don't affect other workspaces
- **Schema independence** - different NodeType versions
- **Parallel development** - multiple teams working simultaneously
- **Environment separation** - dev/staging/prod environments

## Performance Characteristics

### Read Performance
- **O(log n) node lookups** via RocksDB indexing
- **Efficient tree traversal** with optimized relationship storage
- **Query optimization** for common access patterns
- **Caching layer** for frequently accessed data

### Write Performance
- **Batch operations** for multiple node updates
- **Atomic transactions** with minimal overhead
- **Write-ahead logging** for durability
- **Background compaction** for storage optimization

### Scalability
- **Horizontal scaling** through repository sharding
- **Vertical scaling** with efficient resource utilization
- **Storage scaling** with RocksDB's compression and compaction
- **Concurrent access** with fine-grained locking

## Security Model

### Authentication
- **Repository-scoped access** controls
- **Workspace-level permissions** for fine-grained access
- **API key management** for programmatic access
- **Audit logging** for compliance and debugging

### Data Integrity
- **Schema validation** prevents invalid data
- **Referential integrity** maintains relationship consistency
- **Transaction isolation** prevents data races
- **Backup verification** ensures data recoverability

## Deployment Architecture

### Single Instance
Perfect for development and small deployments:
- All components in one process
- Local RocksDB storage
- Direct HTTP API access

### Distributed Deployment
For production and high-availability:
- Load-balanced HTTP layer
- Clustered storage backend
- Shared NodeType registry
- Backup and monitoring

## Next Steps

- 🔧 [Explore the REST API](/docs/access/rest/overview)
- 📝 [Define NodeTypes](/docs/model/nodetypes/overview)
- ⚙️ [Installation Guide](/docs/getting-started/installation)