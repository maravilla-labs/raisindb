# Transaction and Commit API

## Overview

RaisinDB implements a Git-like revision system with two modes of operation:

- **Draft Mode**: Regular PUT/POST/DELETE operations update mutable HEAD (no revision created)
- **Commit Mode**: Explicit commit operations create immutable repository revisions

This enables workflows like:
- ✅ **Real-time collaboration**: Draft changes without creating revisions
- ✅ **Audit trail**: Commit important milestones with messages  
- ✅ **Rollback capability**: Restore to any previous revision
- ✅ **Branching**: Parallel development with merge workflows

## Architecture

```
Draft Operations (PUT/POST/DELETE)    Commit Operations (POST raisin:cmd/*)
         ↓                                        ↓
   Mutable HEAD                           Immutable Revision
   (no revision)                          (creates revision++)
   Fast, immediate                        Atomic, traceable
```

### Key Concepts

1. **Mutable HEAD**: The current working state, modified by draft operations
2. **Commit**: Creates an immutable revision snapshot with a message and actor
3. **Revision**: An immutable point-in-time snapshot of repository state
4. **Transaction**: A collection of operations to be committed atomically

### Draft vs Commit

| Aspect | Draft Mode | Commit Mode |
|--------|------------|-------------|
| **Speed** | Instant | Slightly slower |
| **Revision** | ❌ No | ✅ Yes |
| **Message** | ❌ No | ✅ Required |
| **Audit Trail** | Limited | Full |
| **Rollback** | Manual undo | Change branch HEAD |
| **Use Case** | Development, editing | Deployments, releases |

## HTTP API Endpoints

### 1. Batch Commit (Multi-Node)

Creates a revision with multiple operations committed atomically.

**Endpoint**: `POST /api/repository/{repo}/{branch}/{workspace}/raisin:cmd/commit`

**Request Body**:
```json
{
  "message": "Commit message describing the changes",
  "actor": "user-id or system identifier",
  "operations": [
    {
      "type": "create",
      "node": {
        "id": "new-node-123",
        "name": "new-page",
        "path": "/content/new-page",
        "node_type": "raisin:Page",
        "properties": {
          "title": "New Page Title"
        },
        "version": 1,
        "children": []
      }
    },
    {
      "type": "update",
      "node_id": "existing-node-456",
      "properties": {
        "title": "Updated Title",
        "author": "John Doe"
      }
    },
    {
      "type": "delete",
      "node_id": "old-node-789"
    },
    {
      "type": "move",
      "node_id": "node-to-move",
      "new_parent_path": "/new/parent/location"
    }
  ]
}
```

**Response** (200 OK):
```json
{
  "revision": 42,
  "operations_count": 4
}
```

**Error Responses**:
- `400 Bad Request`: Empty operations, missing message, or invalid operation format
- `404 Not Found`: Referenced node not found
- `500 Internal Server Error`: Storage or system error

### Operation Types

#### Create Operation
Creates a new node in the transaction.

```json
{
  "type": "create",
  "node": {
    "id": "unique-id",
    "name": "node-name",
    "path": "/parent/path/node-name",
    "node_type": "raisin:Page",
    "properties": { ... },
    "version": 1,
    "children": []
  }
}
```

#### Update Operation
Updates properties of an existing node.

```json
{
  "type": "update",
  "node_id": "existing-node-id",
  "properties": {
    "title": "New Title",
    "description": "Updated description"
  }
}
```

#### Delete Operation
Deletes a node.

```json
{
  "type": "delete",
  "node_id": "node-to-delete"
}
```

#### Move Operation
Moves a node to a new parent location.

```json
{
  "type": "move",
  "node_id": "node-to-move",
  "new_parent_path": "/new/parent"
}
```

### 2. Single-Node Commit (GitHub-Like Pattern)

For convenience, RaisinDB provides GitHub-style single-node commit endpoints. These are shortcuts that create a transaction with one operation and immediately commit it.

#### Save (Update with Commit)

Update a single node and create a revision in one request.

**Endpoint**: `POST /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/save`

**Request Body**:
```json
{
  "message": "Update homepage title",
  "actor": "alice",
  "operations": [
    {
      "type": "update",
      "node_id": "homepage-id",
      "properties": {
        "title": "New Homepage Title",
        "updated": true
      }
    }
  ]
}
```

**Response**:
```json
{
  "revision": 42,
  "operations_count": 1
}
```

**Example**:
```bash
# Update a blog post with commit message
curl -X POST "http://localhost:8080/api/repository/blog/main/prod/content/posts/my-post/raisin:cmd/save" \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Fix typos in introduction",
    "actor": "editor@example.com",
    "operations": [{
      "type": "update",
      "node_id": "post-123",
      "properties": {
        "content": "Updated content without typos"
      }
    }]
  }'
```

#### Create (Create with Commit)

Create a new node and commit in one request.

**Endpoint**: `POST /api/repository/{repo}/{branch}/{workspace}/raisin:cmd/create`

**Request Body**:
```json
{
  "message": "Add new contact page",
  "actor": "cms-bot",
  "operations": [
    {
      "type": "create",
      "node": {
        "id": "contact-page",
        "name": "contact",
        "path": "/pages/contact",
        "node_type": "raisin:Page",
        "properties": {
          "title": "Contact Us",
          "content": "Get in touch..."
        },
        "version": 1,
        "children": []
      }
    }
  ]
}
```

**Response**:
```json
{
  "revision": 43,
  "operations_count": 1
}
```

#### Delete (Delete with Commit)

Delete a node and create a revision with audit trail.

**Endpoint**: `POST /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/delete`

**Request Body**:
```json
{
  "message": "Remove obsolete pricing page",
  "actor": "product-manager"
}
```

**Response**:
```json
{
  "revision": 44,
  "operations_count": 1
}
```

**Example**:
```bash
# Delete with commit message (creates audit trail)
curl -X POST "http://localhost:8080/api/repository/website/main/prod/content/old-page/raisin:cmd/delete" \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Remove deprecated content per Q4 cleanup",
    "actor": "admin@example.com"
  }'
```

### Draft vs Commit Comparison

| Operation | Draft Endpoint | Commit Endpoint | Revision Created? |
|-----------|---------------|-----------------|-------------------|
| **Create** | `POST /content/parent-path` | `POST /raisin:cmd/create` | ❌ / ✅ |
| **Update** | `PUT /content/node-path` | `POST /node-path/raisin:cmd/save` | ❌ / ✅ |
| **Delete** | `DELETE /content/node-path` | `POST /node-path/raisin:cmd/delete` | ❌ / ✅ |
| **Batch** | Multiple PUTs | `POST /raisin:cmd/commit` | ❌ / ✅ |

**Use Draft When:**
- 👤 Real-time editing in UI
- 🚀 Need instant feedback  
- 🔄 Frequent autosaves
- 🎨 Experimentation / prototyping

**Use Commit When:**
- 📝 Important milestones
- 🏭 Production deployments
- 📊 Audit requirements
- ↩️ Need rollback capability

## Code Examples

### Server-side (Rust)

```rust
use raisin_core::prelude::*;

// Create connection
let connection = RaisinConnection::open("./data").await?;
let tenant = connection.tenant("acme-corp");
let repo = tenant.repository("website");
let workspace = repo.workspace("main");

// Start a transaction
let mut tx = workspace.nodes().branch("develop").transaction();

// Add operations
tx.create(new_node);
tx.update(node_id, properties);
tx.delete(old_node_id);
tx.move_node(node_id, new_parent_path);

// Commit (creates revision)
let revision = tx.commit("Bulk content update", "user-123").await?;
println!("Created revision: {}", revision);

// Or rollback (discard changes)
tx.rollback();
```

### Client-side (HTTP)

```bash
# Commit multiple operations
curl -X POST "http://localhost:8080/api/repository/default/main/demo/raisin:cmd/commit" \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Update homepage and add new section",
    "actor": "user-123",
    "operations": [
      {
        "type": "update",
        "node_id": "homepage",
        "properties": {
          "title": "Welcome to Our Site"
        }
      },
      {
        "type": "create",
        "node": {
          "id": "new-section",
          "name": "new-section",
          "path": "/sections/new-section",
          "node_type": "raisin:Folder",
          "properties": {},
          "version": 1,
          "children": []
        }
      }
    ]
  }'
```

Response:
```json
{
  "revision": 15,
  "operations_count": 2
}
```

### JavaScript/TypeScript

```typescript
// Transaction helper
async function commitTransaction(
  repo: string,
  branch: string,
  workspace: string,
  operations: TxOperation[],
  message: string,
  actor: string
) {
  const response = await fetch(
    `/api/repository/${repo}/${branch}/${workspace}/raisin:cmd/commit`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        message,
        actor,
        operations
      })
    }
  );

  if (!response.ok) {
    throw new Error(`Commit failed: ${response.statusText}`);
  }

  return await response.json();
}

// Usage
const result = await commitTransaction(
  'default',
  'main',
  'demo',
  [
    {
      type: 'create',
      node: {
        id: 'blog-post-1',
        name: 'first-post',
        path: '/blog/first-post',
        node_type: 'raisin:Page',
        properties: {
          title: 'My First Blog Post',
          author: 'Jane Doe'
        },
        version: 1,
        children: []
      }
    }
  ],
  'Add first blog post',
  'user-jane'
);

console.log(`Created revision ${result.revision}`);
```

## Workflows

### Development → Staging → Production

```bash
# 1. Make changes on develop branch (mutable HEAD)
curl -X PUT "http://localhost:8080/api/repository/myrepo/develop/main/page-1" \
  -H "Content-Type: application/json" \
  -d '{"name":"page-1","node_type":"raisin:Page","properties":{"title":"Draft Title"}}'

# 2. Test changes...

# 3. Commit when ready (creates revision on develop)
curl -X POST "http://localhost:8080/api/repository/myrepo/develop/main/raisin:cmd/commit" \
  -d '{"message":"Feature complete","actor":"dev-123","operations":[...]}'

# 4. Promote to staging (cherry-pick or merge)
curl -X POST "http://localhost:8080/api/repository/myrepo/develop/main/page-1/raisin:cmd/publish" \
  -d '{"target_branch":"staging"}'

# 5. Promote to production
curl -X POST "http://localhost:8080/api/repository/myrepo/staging/main/page-1/raisin:cmd/publish" \
  -d '{"target_branch":"production"}'
```

### Batch Content Updates

```bash
# Create multiple nodes in one atomic commit
curl -X POST "http://localhost:8080/api/repository/default/main/demo/raisin:cmd/commit" \
  -H "Content-Type: application/json" \
  -d '{
    "message": "Import initial content",
    "actor": "admin",
    "operations": [
      {"type":"create","node":{...}},
      {"type":"create","node":{...}},
      {"type":"create","node":{...}}
    ]
  }'
```

## Best Practices

### When to Use Regular Operations vs Commits

**Regular Operations (PUT/POST)** - Updates mutable HEAD:
- Quick drafts and edits
- Work-in-progress changes
- Frequent autosaves
- Development iterations

**Commits (raisin:cmd/commit)** - Creates immutable revision:
- Completed features
- Milestone snapshots
- Release preparation
- Batch imports/exports
- Audit trail requirements

### Commit Messages

Follow Git-style commit messages:

```
✅ Good:
"Add user authentication pages"
"Fix: Correct pricing calculation in cart"
"Refactor: Reorganize content structure"

❌ Bad:
"Update"
"Changes"
"WIP"
```

### Transaction Size

- **Small commits**: 1-10 operations (preferred)
- **Medium commits**: 10-100 operations (acceptable)
- **Large commits**: >100 operations (consider batching)

### Error Handling

```typescript
try {
  const result = await commitTransaction(repo, branch, ws, ops, msg, actor);
  console.log(`Success: revision ${result.revision}`);
} catch (error) {
  if (error.status === 400) {
    console.error('Invalid operation format or empty transaction');
  } else if (error.status === 404) {
    console.error('Referenced node not found');
  } else {
    console.error('System error:', error);
  }
}
```

## Comparison with Git

| Git | RaisinDB |
|-----|----------|
| `git add` | No staging - operations are in-memory |
| `git commit` | `POST .../raisin:cmd/commit` |
| `git log` | Revision history API (TODO) |
| `git checkout <rev>` | `.revision(42)` time-travel reads |
| `git branch` | Repository branches |
| `git merge` | Publishing between branches |
| `git reset --hard` | Transaction rollback (before commit) |
| `rm file && git add -u` | Draft `DELETE` (no revision) |
| `git rm file && git commit` | Commit delete (creates revision) |

## Delete Behavior & Version History

### Version History Preservation

**Important**: When you delete a node from HEAD, its **version history is preserved**. This enables git-like semantics where deleted content remains accessible in historical revisions.

```
Revision 100 (before delete)  →  Has node X + version history
          ↓
    DELETE node X from HEAD
          ↓
HEAD (after delete)  →  ❌ No node X (current state)
Revision 100         →  ✅ Still has node X (immutable)
Version history      →  ✅ Preserved (can restore)
```

### Draft Delete vs Commit Delete

#### Draft Delete (No Revision)

```bash
# Delete from HEAD without creating revision
DELETE /api/repository/blog/main/prod/content/old-post

# Result:
# - Node removed from HEAD ❌
# - Previous revisions still have node ✅
# - Version history preserved ✅
# - NO audit trail created ❌
# - Fast, immediate ⚡
```

**Use When**: Quick cleanup in development, temporary removals

#### Commit Delete (Creates Revision)

```bash
# Delete with commit message (creates audit trail)
POST /api/repository/blog/main/prod/content/old-post/raisin:cmd/delete
{
  "message": "Remove deprecated Q3 announcement",
  "actor": "content-manager@example.com"
}

# Result:
# - Node removed from HEAD ❌
# - New revision created (101) ✅
# - Revision 100 still has node ✅
# - Version history preserved ✅
# - Full audit trail ✅
# - Can rollback ✅
```

**Use When**: Production changes, compliance requirements, important deletions

### Rollback After Delete

```bash
# Oops, deleted important content at revision 102
# Rollback by updating branch HEAD to revision 101

PUT /api/management/repositories/default/blog/branches/main/head
{
  "head": 101
}

# Now HEAD points to revision 101 (before delete)
# Deleted content is restored! ✅
```

### Delete Across Revisions

Deleted nodes behave like in git:

```
main branch (revision 100):  Has node X
main branch (revision 101):  Node X deleted
feature branch (revision 100): Still has node X (diverged before delete)

# Feature branch can merge the delete:
PUT /api/.../branches/feature/head { "head": 101 }
# Now feature branch also doesn't have node X
```

**Key Points**:
- ✅ Old revisions remain **immutable** - deleted nodes still in history
- ✅ Can **time-travel** to see deleted content  
- ✅ Can **rollback** branch HEAD to restore deleted nodes
- ✅ **Branches can diverge** - one deletes, another keeps
- ✅ Version history **never deleted** - enables data recovery

## Related Documentation

- [Repository Management API](./API_REPOSITORIES.md)
- [Branch & Tag API](./API_BRANCHES_TAGS.md)
- [Node Versioning](./API_NODE_VERSIONS.md)
- [REFACTOR.md](../REFACTOR.md) - Architecture overview

## Future Enhancements

- [ ] Revision history endpoint (`GET /revisions`)
- [ ] Revision metadata (changed nodes, diffs)
- [ ] Merge commit support (multiple parent revisions)
- [ ] Differential snapshots (delta storage)
- [ ] Branch fast-forward operations
- [ ] Conflict detection and resolution
