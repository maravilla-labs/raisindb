# RESTORE NODE

RaisinDB supports restoring nodes to their state at previous revisions, similar to Git's `git restore --source=<commit> path/to/file`. This allows you to undo changes or recover previous content without copying nodes to different locations.

## Overview

RESTORE NODE performs an **in-place restoration**:
- The node stays at its current path
- Its properties are updated to match the historical state
- A new revision is created (restoring is a normal change operation)
- The operation is atomic and consistent

## Syntax

```sql
RESTORE [TREE] NODE <node-reference>
  TO REVISION <revision-ref>
  [TRANSLATIONS ('locale1', 'locale2', ...)]
```

### Parameters

| Parameter | Description |
|-----------|-------------|
| `TREE` | Optional. Restore node and all its descendants |
| `NODE <ref>` | Node to restore (by path or id) |
| `TO REVISION` | Historical revision to restore from |
| `TRANSLATIONS` | Optional. Specific locales to restore (defaults to all) |

## Node Reference Formats

You can identify the node to restore by path or by ID:

```sql
-- By path
RESTORE NODE path='/articles/my-article' TO REVISION HEAD~2

-- By ID
RESTORE NODE id='550e8400-e29b-41d4-a716-446655440000' TO REVISION HEAD~2
```

## Revision Reference Formats

The `TO REVISION` clause supports multiple formats:

```sql
-- HEAD-relative: N revisions before the node's current state
RESTORE NODE path='/article' TO REVISION HEAD~2

-- Branch-relative: HEAD of another branch
RESTORE NODE path='/article' TO REVISION main~0

-- HLC timestamp (Hybrid Logical Clock)
RESTORE NODE path='/article' TO REVISION 1734567890123_42
```

### Revision Semantics

| Reference | Description |
|-----------|-------------|
| `HEAD~0` | Current state (no-op) |
| `HEAD~1` | Previous revision |
| `HEAD~N` | N revisions back in this node's history |
| `branch~0` | State at the head of the specified branch |
| `HLC timestamp` | Exact point-in-time restoration |

## Examples

### Restore Single Node to Previous State

```sql
-- Undo the last change to an article
RESTORE NODE path='/articles/bad-article' TO REVISION HEAD~1

-- Restore to 3 revisions ago
RESTORE NODE path='/articles/old-content' TO REVISION HEAD~3
```

### Restore by Node ID

```sql
-- Useful for nodes that may have been moved
RESTORE NODE id='550e8400-e29b-41d4-a716-446655440000'
  TO REVISION HEAD~2
```

### Restore to Branch State

```sql
-- Restore content to match the main branch
RESTORE NODE path='/articles/diverged-content' TO REVISION main~0
```

### Restore with HLC Timestamp

```sql
-- Restore to exact point in time
RESTORE NODE path='/articles/my-article'
  TO REVISION 1734567890123_42
```

### Restore Subtree (TREE)

Restore an entire subtree including all descendants:

```sql
-- Restore entire section including all children
RESTORE TREE NODE path='/products/category-a' TO REVISION HEAD~5

-- Restore by ID with all children
RESTORE TREE NODE id='550e8400-e29b-41d4-a716-446655440000'
  TO REVISION HEAD~3
```

**Note:** RESTORE TREE runs as a background job since it may be long-running. The command returns immediately with a `job_id` that you can use to track progress:

| Column | Type | Description |
|--------|------|-------------|
| `job_id` | String | Job ID for tracking progress |
| `status` | String | Always "queued" initially |
| `path` | String | Path of the root node being restored |
| `revision` | String | Target revision |

### Selective Translation Restore

Restore only specific translations while keeping others unchanged:

```sql
-- Only restore English translation from previous revision
RESTORE NODE path='/articles/my-article'
  TO REVISION HEAD~2
  TRANSLATIONS ('en')

-- Restore German and French translations only
RESTORE NODE path='/articles/my-article'
  TO REVISION HEAD~3
  TRANSLATIONS ('de', 'fr')

-- Combine with TREE for subtree translation restore
RESTORE TREE NODE path='/products'
  TO REVISION HEAD~5
  TRANSLATIONS ('en', 'de')
```

**Behavior with TRANSLATIONS:**
- Only the specified locales are updated from the historical revision
- Other translations remain unchanged at their current state
- If a specified translation doesn't exist at the historical revision, it's skipped with a warning

## Behavior

1. **In-place restore**: Updates the existing node's properties to match the historical state
2. **Same path**: The node stays at its current path (restore, not move/copy)
3. **Creates new revision**: The restore operation creates a new revision like any other change
4. **No data loss**: The previous state is still accessible via revision history
5. **Translation handling**:
   - Without TRANSLATIONS: restores all properties and translations
   - With TRANSLATIONS: only restores specified locales, keeps others unchanged
6. **TREE operations**: Run as background jobs for large subtrees, returning a `job_id` for progress tracking

## Return Value

### RESTORE NODE (single node)

Returns a single row with the operation result:

| Column | Type | Description |
|--------|------|-------------|
| `result` | String | Human-readable status message |
| `affected_rows` | Integer | Number of nodes restored (always 1) |
| `path` | String | Path of the restored node |
| `revision` | String | The revision that was restored from |

### RESTORE TREE NODE (background job)

Returns immediately with job information:

| Column | Type | Description |
|--------|------|-------------|
| `result` | String | Status message about queued job |
| `job_id` | String | Job ID for tracking progress via `/api/jobs/{job_id}` |
| `status` | String | Always "queued" initially |
| `path` | String | Path of the root node |
| `revision` | String | Target revision |

## Errors

| Error | Cause |
|-------|-------|
| Node not found | The specified node doesn't exist at the current HEAD |
| Revision not found | The node doesn't exist at the specified revision |
| Invalid HLC | The HLC timestamp format is invalid |
| Insufficient history | HEAD~N where N exceeds available revisions |

## Related Commands

- [CREATE BRANCH](./branches.md#create-branch) - Create branches from specific revisions
- [COPY NODE](./raisinsql.md) - Copy nodes to new locations
