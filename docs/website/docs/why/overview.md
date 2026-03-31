---
sidebar_position: 1
---

# Why RaisinDB

Welcome to **RaisinDB** — a multi-model database with Git-grade workflows. This page is aimed at product leaders, architects, and executives who need to understand why RaisinDB matters before diving into the technical guides.

## Who Should Read This?

- **Engineering leaders** evaluating databases for content, knowledge graphs, or AI-heavy applications.
- **Platform teams** looking for Git-style collaboration on structured content.
- **Decision makers** comparing RaisinDB with SurrealDB, PostgreSQL, MongoDB, or CockroachDB.

## Proof Backed by Source

- **Server runtime** — `crates/raisin-server/src/main.rs` exposes multi-tenant configuration, replication flags, and monitoring hooks.
- **Node service layer** — `crates/raisin-core/src/services/node_service/mod.rs` powers tree-structured nodes, validation, and publication.
- **Transport stack** — `crates/raisin-transport-http/src/routes.rs` and `raisin-transport-ws` provide the REST + WebSocket surface.
- **Storage** — `crates/raisin-rocksdb` delivers RocksDB persistence, replication, embeddings, and search indexes.

## What is RaisinDB?

RaisinDB is an open-source multi-model database that combines document storage, graph relationships, vector search, and full-text indexing with git-like version control. It provides a PostgreSQL-compatible SQL interface alongside REST APIs, making it perfect for modern applications that need structured data with versioning capabilities.

## Key Features

### Multi-Model Database
- **SQL Queries** — PostgreSQL-compatible RaisinSQL with hierarchical functions and JSON operators
- **Graph Database** — Bidirectional relationships with `NEIGHBORS()` function
- **Vector Search** — Semantic similarity with `KNN()` for embeddings
- **Full-Text Search** — Tantivy-powered search with 20+ language support

### Git-like Version Control
- **Branches & Tags** — Built-in branching, tagging, and commit workflows
- **Time-Travel Queries** — Query data as it existed at any revision
- **Merge Workflows** — Combine changes from different branches
- **Audit Trails** — Track every change with built-in audit logs

### Schema-Driven Architecture
- **NodeTypes** — YAML-based schema definitions with validation
- **Type Safety** — Catch errors early with strong typing
- **Indexing Control** — Specify which properties to index for SQL and full-text search
- **Repository-First** — Isolated repositories with shared schemas

### Performance & Scalability
- **Built with Rust** — High performance, memory safety, and reliability
- **RocksDB Storage** — Fast key-value storage with atomic transactions
- **Hierarchical Trees** — Efficient path-based queries
- **REST API** — Complete HTTP API for all operations

## Mental Model

RaisinDB uses familiar database concepts:

| Traditional SQL | RaisinDB |
|----------------|----------|
| Database | Repository |
| Table | Workspace |
| Schema/DDL | NodeType |
| Row | Node |
| Column | Property |

```
Repository (Database)
├── Workspace "products" (Table)
│   └── Nodes with NodeType "shop:Product" (Rows with Schema)
└── Workspace "orders" (Table)
    └── Nodes with NodeType "shop:Order"
```

## Quick Start

### Prerequisites

- **Docker** (recommended) or Rust toolchain
- Basic understanding of REST APIs and JSON
- Optional: SQL client for RaisinSQL queries

### Installation

:::info Coming Soon
Installation instructions are being finalized. Check back soon!
:::

### Basic Workflow

1. **Create a repository** (like a database)
2. **Define NodeTypes** (schemas with indexing configuration)
3. **Create workspaces** (like tables)
4. **Insert nodes** (data records)
5. **Query with SQL** or REST API
6. **Search** with full-text or vector similarity
7. **Version control** with branches and commits

## Example: Blog Platform

### 1. Define Schema

```yaml
# blog-article.yaml
name: blog:Article
description: A blog post
properties:
  - name: title
    type: String
    required: true
    indexed_for_sql: true      # Enable SQL filtering
    fulltext_indexed: true      # Enable full-text search
  - name: content
    type: Text
    required: true
    fulltext_indexed: true
  - name: author
    type: Reference
    indexed_for_sql: true
  - name: status
    type: String
    indexed_for_sql: true
  - name: embedding
    type: Vector
    dimensions: 1536
allowed_children: ["raisin:Asset"]
versionable: true
publishable: true
```

### 2. Query with SQL

```sql
-- Find published articles
SELECT id, properties ->> 'title' AS title
FROM nodes
WHERE node_type = 'blog:Article'
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC;

-- Find articles by author (graph query)
SELECT n.*, n.properties ->> 'title' AS title
FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED') AS e
JOIN nodes n ON n.id = e.dst_id
WHERE n.properties ->> 'status' = 'published';

-- Find similar articles (vector search)
SELECT n.*, knn.distance
FROM KNN(:article_embedding, 10) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE n.properties ->> 'status' = 'published'
ORDER BY knn.distance;
```

### 3. Full-Text Search

```http
POST /api/fulltext/search
{
  "workspace_id": "main",
  "query": "rust programming performance",
  "limit": 20
}
```

### 4. Version Control

```bash
# Create a branch for new features
curl -X POST /api/management/repositories/my-blog/branches \
  -d '{"name": "feature/new-layout", "from": "main"}'

# Make changes and commit
curl -X POST /api/repository/my-blog/feature-new-layout/head/workspace/commit \
  -d '{"message": "Update layout", "nodes": [...]}'

# Create a release tag
curl -X POST /api/management/repositories/my-blog/tags \
  -d '{"name": "v1.0", "message": "First release"}'
```

## Core Concepts Overview

### Repositories
- Primary isolation boundary (≈ database)
- Contain workspaces, NodeTypes, and version history
- Separate tenants/projects/products

### Workspaces
- Logical grouping of nodes (≈ table)
- Share NodeTypes within the repository
- Separate environments or content types

### Nodes
- Individual data records with:
  - Properties (JSONB)
  - Tree relationships (hierarchical)
  - Graph relationships (named edges)
  - Optional vector embeddings

### NodeTypes
- YAML schema definitions (≈ table schema)
- Define structure, types, validation
- **Control indexing** with `indexed_for_sql` and `fulltext_indexed`
- Shared across all workspaces in a repository

## Querying Options

| Interface | Use Case | Example |
|-----------|----------|---------|
| **RaisinSQL** | Complex queries, joins, aggregations | `SELECT * FROM nodes WHERE...` |
| **Query DSL** | Programmatic REST API queries | `{"and": [{"field": {"nodeType": ...}}]}` |
| **Full-Text** | Text search across content | `"rust programming"` |
| **Vector Search** | Semantic similarity | `KNN(:embedding, 20)` |

## Next Steps

1. 📖 [Learn the core concepts](/docs/why/concepts) - Data models and architecture
2. 🔍 [Explore querying](/docs/access/sql/overview) - SQL, graph, vector, and full-text search
3. 🏗️ [Understand the architecture](/docs/why/architecture) - System design
4. 📝 [Define NodeTypes](/docs/model/nodetypes/overview) - Schema definitions
5. 🔧 [Explore the REST API](/docs/access/rest/overview) - HTTP endpoints

## Need Help?

- 📚 Browse the [documentation](/docs/why/concepts)
- 🐛 [Report issues](https://github.com/maravilla-labs/raisindb/issues)
- 💡 [Request features](https://github.com/maravilla-labs/raisindb/issues/new)
