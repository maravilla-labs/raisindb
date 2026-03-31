# Delete Behavior in Git-Like Architecture

## Executive Summary

**TL;DR**: Deletes in RaisinDB work like git - deleting from HEAD doesn't break history. Old revisions remain immutable and still contain the deleted nodes. However, we need to enhance the REST API to support GitHub-like "save vs commit" workflows for better UX.

## Current Delete Behavior

### 1. Regular DELETE Operation (Draft Mode)

```bash
DELETE /api/repository/{repo}/{branch}/{workspace}/content/{node-id}
```

**What Happens:**
1. Deletes node from **mutable HEAD** (current working state)
2. Does NOT create a revision
3. Previous revisions remain **immutable** - still contain the node
4. Cascade deletes all version history for that node ID
5. Prevents deletion if node or descendants are published

**Storage Flow:**
```rust
// Service layer (node_service.rs)
pub async fn delete(&self, id: &str) -> Result<bool> {
    // 1. Get node and check published status
    let before = storage.get(id).await?;
    if node.published_at.is_some() {
        return Err("Cannot delete published node");
    }
    
    // 2. Delete from current HEAD
    storage.nodes().delete(id).await?;
    
    // 3. Cascade: remove all versions
    storage.versioning().delete_all_versions(id).await;
    
    // 4. Audit log
    audit.log_delete(node).await;
}
```

### 2. Transaction DELETE Operation (Commit Mode)

```bash
POST /api/repository/{repo}/{branch}/{workspace}/raisin:cmd/commit
{
  "message": "Remove obsolete content",
  "actor": "alice",
  "operations": [
    {
      "type": "delete",
      "node_id": "node-123"
    }
  ]
}
```

**What Happens:**
1. Queues delete operation in transaction
2. On commit: applies delete AND creates immutable revision
3. New revision snapshots workspace state **without the deleted node**
4. Previous revisions still have the node (time-travel works)

**Storage Flow:**
```rust
// Transaction layer (transaction.rs)
impl Transaction {
    pub fn delete(&mut self, node_id: String) {
        self.operations.push(TxOperation::Delete { node_id });
    }
    
    pub async fn commit(self, message: &str, actor: &str) -> Result<u64> {
        let ctx = storage.begin_context().await?;
        
        for op in self.operations {
            match op {
                TxOperation::Delete { node_id } => {
                    ctx.delete_node(node_id).await?; // Atomic delete
                }
            }
        }
        
        // Create immutable revision (snapshots current state)
        let revision = ctx.commit(message, actor).await?;
        Ok(revision)
    }
}
```

## Git-Like Architecture Implications

### Does Delete Break Git Structure? ❌ NO

| Operation | HEAD State | Revision 100 (before delete) | Revision 101 (after delete) |
|-----------|------------|------------------------------|----------------------------|
| **Before** | Has node X | Has node X | - |
| **Draft Delete** | ❌ No node X | ✅ Has node X | - |
| **Commit** | ❌ No node X | ✅ Has node X | ❌ No node X |

**Key Points:**
1. ✅ Old revisions remain **immutable** - they still have the deleted node
2. ✅ You can **time-travel** to see deleted content
3. ✅ You can **rollback** to restore deleted nodes
4. ✅ Branches can diverge - one keeps node, another deletes it
5. ❌ Draft delete affects HEAD only (no revision created)
6. ✅ Transaction commit creates revision snapshot

### Comparison with Git

| Git | RaisinDB | Same? |
|-----|----------|-------|
| `rm file.txt` (working dir) | Draft DELETE | ✅ Mutable state |
| `git add file.txt` + `git commit -m "Remove file"` | Transaction DELETE + commit | ✅ Creates commit |
| `git log` shows file in history | Revision 100 still has node | ✅ History preserved |
| `git checkout <old-commit>` | Read at revision 100 | ✅ Time-travel |
| `git revert <commit>` | Update branch HEAD to old revision | ✅ Rollback |

## Browser Workflow: Save vs Commit

### Current Issue: Missing UX Pattern

**User Story:**
> "I'm editing a blog post in the browser. I want to **save my draft** (like Notion/Google Docs), but also **commit milestones** (like GitHub). How do I choose?"

**Problem:** Our API doesn't have a clear "save draft vs commit" pattern exposed to the REST layer.

### Proposed GitHub-Like Pattern

GitHub's file editor has two buttons:
1. **"Commit directly to main"** → Creates commit immediately
2. **"Create a new branch"** → Saves to feature branch

We should adopt a similar pattern:

#### Option A: Dual-Endpoint Pattern (Recommended)

```bash
# 1. DRAFT MODE: Save to HEAD (no revision)
PUT /api/repository/{repo}/{branch}/{workspace}/content/my-post
{
  "type": "document",
  "properties": {
    "title": "My Blog Post",
    "content": "Work in progress..."
  }
}
→ Updates HEAD, no revision created
→ Fast, immediate feedback
→ Like autosave in Google Docs
```

```bash
# 2. COMMIT MODE: Create revision with message
POST /api/repository/{repo}/{branch}/{workspace}/content/my-post/raisin:cmd/commit
{
  "message": "Add blog post about git-like architecture",
  "actor": "alice",
  "node": {
    "type": "document",
    "properties": {
      "title": "My Blog Post",
      "content": "Final version!"
    }
  }
}
→ Creates revision
→ Explicit milestone
→ Like "Commit changes" in GitHub
```

#### Option B: Query Parameter Pattern

```bash
# Draft save
PUT /api/.../content/my-post?mode=draft

# Commit save
PUT /api/.../content/my-post?mode=commit&message=Release+v1.0&actor=alice
```

**Pros:** Single endpoint, simpler routing  
**Cons:** Mixing concerns, harder to document

#### Option C: Header Pattern

```bash
PUT /api/.../content/my-post
X-Raisin-Mode: commit
X-Raisin-Message: Release v1.0
X-Raisin-Actor: alice
```

**Pros:** RESTful, doesn't pollute URL  
**Cons:** Headers less discoverable, harder to test with cURL

### Recommendation: **Option A (Dual-Endpoint)**

**Rationale:**
1. ✅ **Explicit intent**: Different URLs = different semantics
2. ✅ **Self-documenting**: `/raisin:cmd/commit` clearly creates revision
3. ✅ **GitHub-familiar**: Developers understand "commit" metaphor
4. ✅ **Flexible**: Can mix drafts and commits in same workflow
5. ✅ **Performance**: Drafts skip expensive revision creation

## Delete in Browser Context

### Scenario: User Deletes Node in Admin Console

**Current Behavior:**
```typescript
// Admin console: Delete button clicked
DELETE /api/repository/blog/main/prod/posts/post-123
→ Removes from HEAD immediately
→ No revision created
→ No commit message required
```

**Problems:**
1. ❌ No audit trail (no commit message)
2. ❌ Can't undo easily (no revision to rollback to)
3. ❌ Destructive - CASCADE deletes all versions
4. ❌ Bypasses review workflows

### Proposed: Delete Should Require Commit

**Option 1: Force commit on delete**
```typescript
// Admin console: Delete button opens modal
POST /api/repository/blog/main/prod/raisin:cmd/commit
{
  "message": "Remove outdated post-123",
  "actor": "alice",
  "operations": [
    {
      "type": "delete",
      "node_id": "post-123"
    }
  ]
}
→ Creates revision
→ Audit trail preserved
→ Can rollback if mistake
```

**Option 2: Two-stage delete**
```typescript
// Step 1: Mark for deletion (draft)
PUT /api/repository/blog/main/prod/posts/post-123
{
  "properties": {
    "_marked_for_deletion": true,
    "_deleted_by": "alice",
    "_deleted_at": "2025-10-14T10:00:00Z"
  }
}

// Step 2: Commit the deletion later
POST /api/.../raisin:cmd/commit
{
  "message": "Batch cleanup: Remove 5 deprecated posts",
  "operations": [
    {"type": "delete", "node_id": "post-123"},
    {"type": "delete", "node_id": "post-456"},
    ...
  ]
}
```

## Issues Found & Recommendations

### 🔴 Issue 1: Draft Delete is Destructive

**Problem:**
```rust
// service/node_service/mod.rs
pub async fn delete(&self, id: &str) -> Result<bool> {
    storage.nodes().delete(id).await?;
    
    // ⚠️ CASCADE DELETE: Removes ALL version history
    storage.versioning().delete_all_versions(id).await;
}
```

**Why This is Bad:**
- If you draft-delete a node, **all its version history is gone forever**
- Can't restore from versions
- Git doesn't work this way (deleted files still in history)

**Fix:**
```rust
pub async fn delete(&self, id: &str) -> Result<bool> {
    // Only delete from HEAD
    storage.nodes().delete(id).await?;
    
    // ✅ KEEP version history - it's immutable!
    // Don't cascade delete versions here
    
    // Versions will be inaccessible from HEAD, but still exist
    // Can be restored if needed
}
```

**Rationale:** Version history is **metadata about past states**, not tied to current HEAD. Deleting HEAD shouldn't destroy history.

### 🔴 Issue 2: Missing Commit Endpoint for Single-Node Operations

**Current State:**
- ✅ Multi-node commits: `POST .../raisin:cmd/commit` with operations array
- ❌ Single-node commits: No direct endpoint

**User Expectation (GitHub-like):**
```bash
# User edits a file in GitHub UI, clicks "Commit changes"
# This creates a commit for THAT SINGLE FILE, not a batch

# Equivalent in RaisinDB?
POST /api/repository/blog/main/prod/posts/my-post/raisin:cmd/save
{
  "message": "Update blog post title",
  "actor": "alice",
  "properties": {
    "title": "New Title"
  }
}
→ Should update node AND create revision in one atomic operation
```

**Current Workaround (Clunky):**
```bash
# Step 1: Update node (draft)
PUT /api/repository/blog/main/prod/posts/my-post
{ "properties": { "title": "New Title" } }

# Step 2: Commit separately
POST /api/repository/blog/main/prod/raisin:cmd/commit
{
  "message": "Update blog post title",
  "operations": [
    {
      "type": "update",
      "node_id": "my-post",
      "properties": { "title": "New Title" }
    }
  ]
}
```

**Recommendation: Add single-node commit endpoint**

```rust
// handlers/nodes.rs
pub async fn commit_node_update(
    State(state): State<AppState>,
    Path((tenant, repo, branch, ws, path)): Path<(String, String, String, String, String)>,
    Json(req): Json<CommitNodeRequest>,
) -> Result<Json<CommitResponse>, StatusCode> {
    let mut tx = state.connection()
        .tenant(&tenant)
        .repository(&repo)
        .workspace(&ws)
        .nodes()
        .branch(&branch)
        .transaction();
    
    tx.update(path, req.properties);
    let revision = tx.commit(req.message, req.actor).await?;
    
    Ok(Json(CommitResponse { revision }))
}
```

**Route:**
```
POST /api/repository/{repo}/{branch}/{ws}/content/{path}/raisin:cmd/save
```

### 🟡 Issue 3: Delete Doesn't Respect Revisions

**Problem:** When you delete via transaction, the node is gone from ALL future reads at HEAD. But what if another branch still needs it?

**Scenario:**
```
main branch (revision 100): Has node X
feature branch (revision 100): Has node X

# User commits delete on main
POST /main/raisin:cmd/commit
{ "operations": [{"type": "delete", "node_id": "X"}] }
→ Creates revision 101 on main

main (revision 101): ❌ No node X
feature (revision 100): ✅ Still has node X

# Feature branch updates HEAD to 101
PUT /api/repository/blog/feature
{ "head": 101 }

feature (revision 101): ❌ No node X (inherited delete)
```

**This is CORRECT git-like behavior!** But needs documentation.

### 🟢 Issue 4: Positive - Rollback Works Correctly

**Scenario:**
```bash
# Accidentally delete important content
POST /api/.../raisin:cmd/commit
{
  "message": "Clean up old files",
  "operations": [
    {"type": "delete", "node_id": "important-data"}
  ]
}
→ Creates revision 102 (no important-data)

# Realize mistake, rollback
PUT /api/repository/blog/main
{ "head": 101 }  # Point back to revision before delete

# Now reads show important-data again!
GET /api/repository/blog/main/prod/content/important-data
→ 200 OK (node restored)
```

✅ **This works perfectly** because revisions are immutable.

## Recommendations Summary

### 1. Fix Cascade Delete (CRITICAL)

```diff
// crates/raisin-core/src/services/node_service/mod.rs
pub async fn delete(&self, id: &str) -> Result<bool> {
    let res = self.storage.nodes().delete(...).await?;
    if res {
-       // Cascade delete: remove all versions for this node
-       let _ = self.storage.versioning().delete_all_versions(id).await;
+       // Keep version history - it's immutable metadata
+       // Versions remain for historical queries and rollback
    }
}
```

### 2. Add Single-Node Commit Endpoints

Create shortcuts for common GitHub-like workflows:

```rust
// New handler: Commit single node update
POST /api/repository/{repo}/{branch}/{ws}/content/{path}/raisin:cmd/save
{
  "message": "Update title",
  "actor": "alice",
  "properties": { "title": "New" }
}

// New handler: Commit single node delete
DELETE /api/repository/{repo}/{branch}/{ws}/content/{path}/raisin:cmd/delete
{
  "message": "Remove obsolete content",
  "actor": "alice"
}
→ Returns { "revision": 103 }
```

### 3. Update REST API Documentation

Add section to `docs/API_TRANSACTIONS.md`:

```markdown
## Delete Behavior

### Draft Delete (No Revision)
DELETE /api/repository/{repo}/{branch}/{ws}/content/{path}
- Removes from HEAD immediately
- No commit message required
- No revision created
- Version history PRESERVED (can rollback)

### Commit Delete (Creates Revision)
POST /api/repository/{repo}/{branch}/{ws}/content/{path}/raisin:cmd/delete
{
  "message": "Remove X because Y",
  "actor": "username"
}
- Creates immutable revision
- Audit trail preserved
- Can rollback to previous revision
- Recommended for production workflows
```

### 4. Admin Console UX

**Current:**
```
[Delete] button → Immediate delete, no confirm
```

**Proposed:**
```
[Delete] button → Modal opens:
  
  Delete "Homepage" permanently?
  
  ( ) Delete from draft (immediate)
  (•) Commit deletion (create revision)
  
  Commit message: [Remove obsolete homepage___________]
  
  [Cancel] [Delete]
```

### 5. Document Revision Behavior

Create `docs/REVISION_SEMANTICS.md` explaining:
- How deletes work across revisions
- Branch inheritance of deletes
- Rollback strategies
- Best practices

## Next Steps

1. ✅ **Document current behavior** (this file)
2. 🔴 **Fix cascade delete issue** (remove version deletion)
3. 🟡 **Add single-node commit endpoints** (GitHub-like UX)
4. 🟢 **Update API docs** with delete semantics
5. 🟢 **Enhance admin console** with commit option
6. 🟢 **Add integration tests** for delete scenarios

## Conclusion

**Does delete break git-like structure?** ❌ **NO**

- Old revisions remain immutable
- Time-travel works correctly
- Rollback restores deleted content
- Branches can diverge (one deletes, another keeps)

**Main Issues Found:**
1. ✅ **Cascade delete is too aggressive** - should preserve version history
2. ✅ **Missing single-node commit pattern** - need GitHub-like "save with message" 
3. ✅ **REST API needs clarity** on draft vs commit semantics

**Next Priority:** Fix cascade delete and add single-node commit endpoints to match user expectations from GitHub/GitLab workflows.
