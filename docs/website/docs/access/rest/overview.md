---
sidebar_position: 1
---

# API Reference

RaisinDB exposes a comprehensive REST API for managing repositories, workspaces, nodes, and NodeTypes. This reference documents the actual endpoints as implemented in the `raisin-transport-http` crate.

## Base URL & Conventions

- **Base prefix:** `/api`
- **Path parameters:** Curly braces `{param}`
- **Wildcard paths:** `{*node_path}` for hierarchical content
- **ID references:** Use `$ref/{id}` syntax for direct ID access
- **Time contexts:**
  - `head` — current, mutable state
  - `rev/{revision}` — historical snapshot, read-only

---

## 📦 Workspaces

Workspaces are logical groupings within a repository (similar to collections or tables).

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/workspaces/{repo}` | List all workspaces in a repository |
| GET | `/api/workspaces/{repo}/{name}` | Get a specific workspace |
| PUT | `/api/workspaces/{repo}/{name}` | Create or update a workspace |
| GET | `/api/workspaces/{repo}/{name}/config` | Get workspace configuration |
| PUT | `/api/workspaces/{repo}/{name}/config` | Update workspace configuration |

---

## 🔍 Query Endpoints

Execute queries against workspace data.

### HEAD (current state)

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/repository/{repo}/{branch}/head/{ws}/query` | JSON filter query |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/query/dsl` | DSL-based query |

---

## 📄 Content Operations

CRUD operations on nodes within a workspace.

### HEAD (mutable)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/repository/{repo}/{branch}/head/{ws}/` | Get root node |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/` | Create at root |
| GET | `/api/repository/{repo}/{branch}/head/{ws}/$ref/{id}` | Fetch node by ID |
| GET | `/api/repository/{repo}/{branch}/head/{ws}/{*node_path}` | Get node by path |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/{*node_path}` | Create node at path |
| PUT | `/api/repository/{repo}/{branch}/head/{ws}/{*node_path}` | Update node at path |
| DELETE | `/api/repository/{repo}/{branch}/head/{ws}/{*node_path}` | Delete node at path |

### Revision (read-only)

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/repository/{repo}/{branch}/rev/{revision}/{ws}/` | Get root at revision |
| GET | `/api/repository/{repo}/{branch}/rev/{revision}/{ws}/$ref/{id}` | Fetch by ID at revision |
| GET | `/api/repository/{repo}/{branch}/rev/{revision}/{ws}/{*node_path}` | Get by path at revision |

---

## 📜 Audit Trails

Track change history for nodes.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/audit/{repo}/{branch}/{ws}/by-id/{id}` | Audit trail by node ID |
| GET | `/api/audit/{repo}/{branch}/{ws}/{*node_path}` | Audit trail by path |

---

## 📐 NodeType Management

Define and manage schemas for your documents.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/management/{repo}/{branch}/nodetypes` | List all NodeTypes |
| POST | `/api/management/{repo}/{branch}/nodetypes` | Create a new NodeType |
| GET | `/api/management/{repo}/{branch}/nodetypes/published` | List published NodeTypes |
| POST | `/api/management/{repo}/{branch}/nodetypes/validate` | Validate a node against a type |
| GET | `/api/management/{repo}/{branch}/nodetypes/{name}` | Get a specific NodeType |
| PUT | `/api/management/{repo}/{branch}/nodetypes/{name}` | Update a NodeType |
| DELETE | `/api/management/{repo}/{branch}/nodetypes/{name}` | Delete a NodeType |
| GET | `/api/management/{repo}/{branch}/nodetypes/{name}/resolved` | Get resolved NodeType (with inheritance) |
| POST | `/api/management/{repo}/{branch}/nodetypes/{name}/publish` | Publish a NodeType |
| POST | `/api/management/{repo}/{branch}/nodetypes/{name}/unpublish` | Unpublish a NodeType |

---

## 🌿 Branch Management

Git-style branches for your repository.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/branches` | List all branches |
| POST | `/api/management/repositories/{tenant_id}/{repo_id}/branches` | Create a new branch |
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}` | Get a specific branch |
| DELETE | `/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}` | Delete a branch |
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head` | Get branch HEAD |
| PUT | `/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head` | Update branch HEAD |

---

## 🏷️ Tag Management

Create immutable tags for revisions.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/tags` | List all tags |
| POST | `/api/management/repositories/{tenant_id}/{repo_id}/tags` | Create a new tag |
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/tags/{name}` | Get a specific tag |
| DELETE | `/api/management/repositories/{tenant_id}/{repo_id}/tags/{name}` | Delete a tag |

---

## 📚 Revision History

Access historical snapshots and changes.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/revisions` | List all revisions |
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/revisions/{revision}` | Get a specific revision |
| GET | `/api/management/repositories/{tenant_id}/{repo_id}/revisions/{revision}/changes` | Get changes in a revision |

---

## 🗄️ Repository Management

Manage repositories at the tenant level.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/repositories` | List all repositories |
| POST | `/api/repositories` | Create a new repository |
| GET | `/api/repositories/{repo_id}` | Get a specific repository |
| PUT | `/api/repositories/{repo_id}` | Update a repository |
| DELETE | `/api/repositories/{repo_id}` | Delete a repository |

---

## 🌐 Registry Management

Manage tenants and deployments.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/management/registry/tenants` | List all tenants |
| POST | `/api/management/registry/tenants` | Create a new tenant |
| GET | `/api/management/registry/tenants/{tenant_id}` | Get a specific tenant |
| GET | `/api/management/registry/deployments` | List all deployments |
| POST | `/api/management/registry/deployments` | Create a new deployment |
| GET | `/api/management/registry/deployments/{tenant_id}/{deployment_key}` | Get a specific deployment |

---

## 🌐 Translations

Manage multilingual content with built-in translation support.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/repositories/{repo}/translation-config` | Get translation configuration |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/translate` | Create or update translation |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/list-translations` | List available translations |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/delete-translation` | Delete a translation |
| POST | `/api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/hide-in-locale` | Hide node in specific locale |

**Query with Locale:**
- Add `?lang={locale}` to any GET endpoint to fetch localized content
- Use `LOCALE '{locale}'` in SQL queries for locale-aware results

See the [Translation API](./translations.md) for end-to-end workflows.

---

## 🔐 Authentication & Admin Users *(RocksDB builds)*

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/raisindb/sys/{tenant_id}/auth` | Authenticate and receive a token |
| POST | `/api/raisindb/sys/{tenant_id}/auth/change-password` | Change the current admin password (requires auth middleware) |
| GET/POST | `/api/raisindb/sys/{tenant_id}/admin-users` | List or create admin users |
| GET/PUT/DELETE | `/api/raisindb/sys/{tenant_id}/admin-users/{username}` | Manage a specific admin account |

---

## 🔁 Replication & Sync *(RocksDB builds)*

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/replication/{tenant_id}/{repo_id}/operations` | Fetch operation log |
| POST | `/api/replication/{tenant_id}/{repo_id}/operations/batch` | Apply batched operations |
| GET | `/api/replication/{tenant_id}/{repo_id}/vector-clock` | Inspect replication vector clock |

---

## 🧠 Embeddings Configuration

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET/POST | `/api/tenants/{tenant_id}/embeddings/config` | Get or set tenant-level embedding provider credentials |
| POST | `/api/tenants/{tenant_id}/embeddings/config/test` | Validate embedding provider connectivity |

---

## 🔎 Search & SQL

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/repository/{repo}/{branch}/fulltext/search` | Tantivy full-text search within a repository/branch |
| GET | `/api/search/{repo}` | Hybrid search (full-text + vector rerank) across repositories |
| POST | `/api/sql/{repo}` | Execute RaisinSQL queries via HTTP |

---

## 🧰 Index Maintenance *(RocksDB builds)*

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/admin/management/database/{tenant}/{repo}/fulltext/verify` | Validate full-text index state |
| POST | `/api/admin/management/database/{tenant}/{repo}/fulltext/rebuild` | Rebuild full-text index |
| POST | `/api/admin/management/database/{tenant}/{repo}/fulltext/optimize` | Optimize segments |
| POST | `/api/admin/management/database/{tenant}/{repo}/fulltext/purge` | Purge deleted documents |
| GET | `/api/admin/management/database/{tenant}/{repo}/fulltext/health` | Inspect health metrics |
| POST | `/api/admin/management/database/{tenant}/{repo}/vector/verify` | Verify vector index consistency |
| POST | `/api/admin/management/database/{tenant}/{repo}/vector/rebuild` | Rebuild vector index |
| POST | `/api/admin/management/database/{tenant}/{repo}/vector/regenerate` | Recompute embeddings |
| POST | `/api/admin/management/database/{tenant}/{repo}/vector/optimize` | Optimize HNSW index |
| POST | `/api/admin/management/database/{tenant}/{repo}/vector/restore` | Restore from backup |
| GET | `/api/admin/management/database/{tenant}/{repo}/vector/health` | Vector index health report |

---

## 🪨 RocksDB Global Admin *(RocksDB builds)*

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/admin/management/global/rocksdb/compact` | Force compaction |
| POST | `/api/admin/management/global/rocksdb/backup` | Trigger RocksDB backup |
| GET | `/api/admin/management/global/rocksdb/stats` | Fetch RocksDB stats |

---

## 👥 Tenant Maintenance *(RocksDB builds)*

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/api/admin/management/tenant/{tenant}/cleanup` | Run cleanup tasks for tenant |
| GET | `/api/admin/management/tenant/{tenant}/stats` | Tenant-level storage stats |

---

## Next Steps

- **[Translation API](./translations.md)** — Complete multilingual content guide
- **[Request/Response Shapes](./shapes.md)** — Detailed DTOs and data structures
- **[API Examples](./examples.md)** — Practical curl examples
- **[NodeTypes Reference](../../model/nodetypes/overview.md)** — Schema definitions
