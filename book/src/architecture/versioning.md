# Git-Like Versioning & Commits

RaisinDB implements a **git-like versioning architecture** that separates mutable workspace drafts from immutable repository snapshots. This enables powerful workflows like feature branches, staging environments, and production deployments—all backed by the same content.

## Core Concepts

### Draft vs Commit Model

RaisinDB distinguishes between two types of content operations:

| Operation Type | Creates Revision | Mutates HEAD | Use Case |
|----------------|------------------|--------------|----------|
| **Draft** (PUT/POST) | ❌ No | ✅ Yes | Development, real-time collaboration |
| **Commit** (Transaction) | ✅ Yes | ✅ Yes | Releases, deployments, checkpoints |

**Draft operations** (`PUT`, `POST`, `DELETE` on nodes) update the workspace's current HEAD pointer without creating a snapshot. Think of this as "working directory changes" in git—fast, immediate, collaborative.

**Commit operations** (`POST .../raisin:cmd/commit`) create immutable revisions that snapshot the entire workspace state. These revisions are numbered sequentially (1, 2, 3, ...) and can be tagged, branched, or restored.

### Revisions

Every commit creates a **revision** with:

- **Revision number** (`u64`): Sequential identifier (1, 2, 3, ...)
- **Commit message**: Human-readable description
- **Actor**: Username or system identifier
- **Timestamp**: When the revision was created
- **Parent revision**: Enables revision history traversal

Revisions are **immutable**—once created, they never change. This guarantees:

✅ **Reproducibility**: Revision 42 always returns the same content  
✅ **Auditability**: Full history of who committed what and when  
✅ **Rollback safety**: Restore to any prior state without data loss

### Branches

Branches are **named pointers** to revisions, similar to git:

```
main       → revision 156
staging    → revision 154
feature-x  → revision 140
```

When you commit to a branch, the branch pointer advances to the new revision. Multiple branches can point to the same revision (like after a merge), and you can create new branches from any revision.

### Tags

Tags are **immutable labels** for specific revisions:

```
v1.0.0     → revision 100 (production release)
beta-2024  → revision 87  (beta snapshot)
```

Unlike branches, tags never move once created—they permanently mark a moment in time.

## Transaction API

### Creating Transactions

Transactions accumulate multiple operations before committing them atomically:

```rust
use raisin_core::{Transaction, Node};

// Start a transaction on a specific branch
let mut tx = workspace
    .nodes()
    .branch("main")
    .transaction();

// Queue operations (not applied yet)
tx.create(Node::folder("projects", "/", HashMap::new())).await?;
tx.create(Node::document("readme", "/projects", HashMap::new())).await?;
tx.update("existing-node-id", HashMap::from([
    ("title".to_string(), serde_json::json!("Updated Title")),
])).await?;

// Commit atomically - all or nothing
let revision = tx.commit("Initial project setup", "alice").await?;
println!("Created revision {}", revision);
```

### Operation Types

Transactions support four operation types:

| Operation | Effect | Example |
|-----------|--------|---------|
| **Create** | Add new node | `tx.create(node)` |
| **Update** | Modify properties | `tx.update(id, props)` |
| **Delete** | Remove node | `tx.delete(id)` |
| **Move** | Change parent | `tx.move_node(id, parent)` |

All operations are applied in order during commit. If any operation fails, the entire transaction rolls back.

### Rollback

Discard pending operations without committing:

```rust
let mut tx = workspace.nodes().branch("main").transaction();
tx.create(node1).await?;
tx.create(node2).await?;

// Changed your mind? Just drop it
tx.rollback(); // All operations discarded
```

Rollback is automatic when the transaction is dropped without calling `commit()`.

## HTTP API

### Batch Commit Endpoint

Create a revision with multiple operations via the REST API:

```bash
POST /api/repository/{repo}/{branch}/{workspace}/raisin:cmd/commit
Content-Type: application/json

{
  "message": "Deploy content to production",
  "actor": "deploy-bot",
  "operations": [
    {
      "type": "create",
      "node": {
        "type": "document",
        "name": "index",
        "path": "/",
        "properties": {
          "title": "Home Page",
          "content": "Welcome!"
        }
      }
    },
    {
      "type": "update",
      "node_id": "abc123",
      "properties": {
        "status": "published"
      }
    }
  ]
}
```

**Response:**

```json
{
  "revision": 157,
  "operations_count": 2
}
```

### Single-Node Commit Endpoints (GitHub-Like)

For convenience, RaisinDB provides GitHub-style single-node commit endpoints that create a transaction with one operation and immediately commit it.

#### Save (Update with Commit)

Update a single node and create a revision in one request:

```bash
POST /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/save
Content-Type: application/json

{
  "message": "Fix typo in homepage",
  "actor": "alice",
  "operations": [{
    "type": "update",
    "node_id": "homepage-id",
    "properties": {
      "title": "Welcome to Our Site",
      "updated": true
    }
  }]
}
```

**Response:** `{"revision": 158, "operations_count": 1}`

**Use Case**: Browser editing - user clicks "Save with message" button

#### Create (Create with Commit)

Create a new node and commit in one request:

```bash
POST /api/repository/{repo}/{branch}/{workspace}/raisin:cmd/create
Content-Type: application/json

{
  "message": "Add contact page",
  "actor": "cms-bot",
  "operations": [{
    "type": "create",
    "node": {
      "id": "contact-page",
      "name": "contact",
      "path": "/pages/contact",
      "node_type": "raisin:Page",
      "properties": {
        "title": "Contact Us"
      },
      "version": 1,
      "children": []
    }
  }]
}
```

**Response:** `{"revision": 159, "operations_count": 1}`

#### Delete (Delete with Commit)

Delete a node and create a revision with audit trail:

```bash
POST /api/repository/{repo}/{branch}/{workspace}/content/{path}/raisin:cmd/delete
Content-Type: application/json

{
  "message": "Remove obsolete pricing page",
  "actor": "admin"
}
```

**Response:** `{"revision": 160, "operations_count": 1}`

**Use Case**: Production deletions with full audit trail and rollback capability

### Draft vs Commit Modes

| Aspect | Draft Mode | Commit Mode |
|--------|------------|-------------|
| **Endpoint** | `PUT /content/node` | `POST /node/raisin:cmd/save` |
| **Speed** | ⚡ Instant | Slightly slower |
| **Revision** | ❌ No | ✅ Yes |
| **Message** | ❌ Not required | ✅ Required |
| **Audit Trail** | Limited | Full |
| **Rollback** | Manual undo | Change branch HEAD |
| **Use Case** | Real-time editing, autosave | Deployments, milestones |

**Use Draft When:**
- 👤 Real-time editing in UI
- 🚀 Need instant feedback
- 🔄 Frequent autosaves
- 🎨 Experimentation

**Use Commit When:**
- 📝 Important milestones
- 🏭 Production deployments
- 📊 Audit requirements
- ↩️ Need rollback capability

### Common Workflows

#### Development → Staging → Production

```bash
# Work in development workspace (drafts)
PUT /api/repository/myrepo/main/dev/content/home
{
  "type": "document",
  "name": "home",
  "properties": { "title": "New Home Page" }
}

# Commit to create a revision
POST /api/repository/myrepo/main/dev/raisin:cmd/commit
{
  "message": "Update home page",
  "actor": "alice"
}
# → Creates revision 158

# Update staging branch to point to revision 158
PUT /api/repository/myrepo/staging
{ "head": 158 }

# Test in staging workspace...

# Deploy to production
PUT /api/repository/myrepo/production
{ "head": 158 }
```

#### Feature Branches

```bash
# Create feature branch from current main (revision 100)
POST /api/repository/myrepo/branches
{
  "name": "feature-redesign",
  "from_revision": 100
}

# Work in feature workspace
PUT /api/repository/myrepo/feature-redesign/work/...

# Commit changes
POST /api/repository/myrepo/feature-redesign/work/raisin:cmd/commit
{ "message": "Redesign complete", "actor": "bob" }
# → Creates revision 101

# Merge to main (update main's HEAD)
PUT /api/repository/myrepo/main
{ "head": 101 }
```

## Comparison with Git

| Feature | Git | RaisinDB |
|---------|-----|----------|
| **Drafts** | Working directory | Workspace HEAD (mutable) |
| **Commits** | `git commit` | Transaction commit |
| **Branches** | `git branch` | Named branch pointers |
| **Tags** | `git tag` | Immutable revision labels |
| **History** | `git log` | Revision list |
| **Checkout** | `git checkout <rev>` | Update branch HEAD |
| **Merge** | `git merge` | Update branch pointer (fast-forward) |
| **Delete file** | `rm file && git add -u` | Draft `DELETE` (no revision) |
| **Commit delete** | `git rm && git commit` | `POST .../raisin:cmd/delete` |

**Key Difference**: RaisinDB commits are at the **repository level**, not file-level. A commit snapshots the entire workspace, not individual files.

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
DELETE /api/repository/{repo}/{branch}/{workspace}/content/old-post
```

**Result:**
- Node removed from HEAD ❌
- Previous revisions still have node ✅
- Version history preserved ✅
- NO audit trail created ❌
- Fast, immediate ⚡

**Use When**: Quick cleanup in development, temporary removals

#### Commit Delete (Creates Revision)

```bash
POST /api/repository/{repo}/{branch}/{workspace}/content/old-post/raisin:cmd/delete
{
  "message": "Remove deprecated content",
  "actor": "admin"
}
```

**Result:**
- Node removed from HEAD ❌
- New revision created ✅
- Previous revisions still have node ✅
- Version history preserved ✅
- Full audit trail ✅
- Can rollback ✅

**Use When**: Production changes, compliance requirements, important deletions

### Rollback After Delete

```bash
# Deleted important content at revision 102
# Rollback by updating branch HEAD to revision 101

PUT /api/management/repositories/default/{repo}/branches/main/head
{
  "head": 101
}

# Now HEAD points to revision 101 (before delete)
# Deleted content is restored! ✅
```

### Delete Across Revisions

Deleted nodes behave like in git - branches can diverge:

```
main branch (revision 100):  Has node X
main branch (revision 101):  Node X deleted
feature branch (revision 100): Still has node X (diverged before delete)

# Feature branch can inherit the delete:
PUT /api/.../branches/feature/head { "head": 101 }
# Now feature branch also doesn't have node X
```

**Key Points:**
- ✅ Old revisions remain **immutable** - deleted nodes still in history
- ✅ Can **time-travel** to see deleted content
- ✅ Can **rollback** branch HEAD to restore deleted nodes
- ✅ **Branches can diverge** - one deletes, another keeps
- ✅ Version history **never deleted** - enables data recovery

## Benefits

✅ **Auditability**: Every deployment is traceable to a specific revision  
✅ **Rollback**: Instantly revert to any prior revision  
✅ **Testing**: Test changes in staging before production  
✅ **Collaboration**: Concurrent work via branches  
✅ **Reproducibility**: Revision 42 is always the same content

## Next Steps

- **[Branches & Tags API](../api/branches.md)**: Detailed API reference
- **[Transactions](../guides/transactions.md)**: Advanced transaction patterns
- **[Multi-Tenancy](multi-tenancy.md)**: Isolated versioning per tenant
