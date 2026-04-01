# RaisinDB Documentation

Welcome to RaisinDB - a flexible, high-performance content database built with Rust.

## What is RaisinDB?

RaisinDB is a modern content database designed for building content-heavy applications like CMSs, headless backends, and SaaS platforms. It provides:

- **Flexible Schema**: Define content types with rich property schemas
- **Multi-Tenancy**: Built-in support for isolating tenant data
- **Workspaces**: Organize data into app-specific categories (content, dam, customers, etc.)
- **Multiple Storage Backends**: RocksDB (default), In-Memory, and extensible
- **Rate Limiting**: Per-tenant rate limiting with RocksDB or Redis
- **RESTful API**: HTTP transport layer included
- **Embeddable**: Use as a library in your Rust application

## Quick Example

### Embedded Usage

```rust
use raisin_core::RaisinConnection;
use raisin_rocksdb::RocksDBStorage;
use std::sync::Arc;

// Create storage
let storage = Arc::new(RocksDBStorage::new("./data").unwrap());

// Create connection and scope to tenant/repo/workspace
let conn = RaisinConnection::with_storage(storage);
let nodes = conn.tenant("default")
    .repository("app")
    .workspace("content")
    .nodes();

// Create a node in the "content" workspace
let node = raisin_models::nodes::Node {
    name: "my-page".to_string(),
    node_type: "raisin:Folder".to_string(),
    ..Default::default()
};

let created = nodes.add_node("/", node).await?;
```

### Multi-Tenant SaaS

```rust
use raisin_core::RaisinConnection;
use raisin_context::TenantContext;

// Create connection and scope to a specific tenant
let conn = RaisinConnection::with_storage(storage);
let tenant = conn.tenant("customer-123");

// Each tenant has their own isolated repositories and workspaces
let content_nodes = tenant.repository("app")
    .workspace("content").nodes().list_all().await?;
let dam_assets = tenant.repository("app")
    .workspace("dam").nodes().list_all().await?;
```

## Key Features

### 🚀 **High Performance**
Built on RocksDB for fast, reliable storage with excellent write throughput.

### 🔒 **Multi-Tenant Isolation**
Complete data isolation between tenants with configurable resolution strategies.

### 📊 **Rate Limiting**
Built-in rate limiting with token bucket and sliding window algorithms.

### 🎨 **Flexible Schema**
Define rich content types with validation, constraints, and relationships.

### 🔌 **Pluggable Architecture**
Implement custom storage backends, tenant resolvers, and tier providers.

### 🦀 **Written in Rust**
Memory-safe, fast, and reliable with minimal runtime overhead.

## Use Cases

- **Headless CMS**: Build a content API for websites and mobile apps
- **SaaS Platforms**: Multi-tenant applications with isolated data
- **Document Databases**: Store and query structured/semi-structured data
- **Content Management**: Manage pages, posts, media, and more

## Getting Started

Choose your path:

- [**Quick Start**](getting-started/quickstart.md) - Get up and running in 5 minutes
- [**Embedded Usage**](getting-started/embedded.md) - Use as a library
- [**Standalone Server**](getting-started/standalone.md) - Run as a service
- [**Building a SaaS**](guides/multi-tenant-saas.md) - Multi-tenant tutorial

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│  Your Application / HTTP Server                 │
└─────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────┐
│  NodeService (raisin-core)                      │
│  - Business logic                               │
│  - Validation                                   │
│  - Multi-tenant scoping                         │
└─────────────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────────────┐
│  Storage Trait (raisin-storage)                 │
│  - Abstract interface                           │
│  - Scoped storage wrapper                       │
└─────────────────────────────────────────────────┘
                    │
        ┌───────────┴───────────┐
        ▼                       ▼
┌──────────────────┐    ┌──────────────────┐
│  RocksDB         │    │  InMemory        │
│  Persistent      │    │  Fast testing    │
└──────────────────┘    └──────────────────┘
```

## License

Business Source License 1.1 (BSL-1.1) - see LICENSE file for details.

## Community

- GitHub: [https://github.com/maravilla-labs/raisindb](https://github.com/maravilla-labs/raisindb)
- Issues: [Report bugs and request features](https://github.com/maravilla-labs/raisindb/issues)

---

Ready to get started? Head to the [Quick Start Guide](getting-started/quickstart.md)!
