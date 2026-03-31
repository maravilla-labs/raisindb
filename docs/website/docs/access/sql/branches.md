# Branch Management

RaisinDB supports Git-like branch management through SQL statements. Branches allow you to create isolated workspaces for development, testing, or content staging without affecting your main data.

## Overview

Branches in RaisinDB work similarly to Git branches:
- Each branch is a named reference to a specific revision
- Changes made on one branch don't affect other branches
- Branches can be merged back together
- You can track upstream branches and view divergence

## CREATE BRANCH

Create a new branch from an existing branch or revision.

```sql
CREATE BRANCH 'branch-name'
  [FROM 'source-branch']
  [AT REVISION <revision-ref>]
  [DESCRIPTION 'description']
  [PROTECTED]
  [UPSTREAM 'upstream-branch']
  [WITH HISTORY]
```

### Parameters

| Parameter | Description |
|-----------|-------------|
| `branch-name` | Name of the new branch (quoted or unquoted) |
| `FROM` | Source branch to create from (optional, creates orphan if omitted) |
| `AT REVISION` | Specific revision to branch from (optional, uses HEAD if omitted) |
| `DESCRIPTION` | Human-readable description of the branch |
| `PROTECTED` | Mark branch as protected (prevents deletion) |
| `UPSTREAM` | Set upstream branch for divergence tracking |
| `WITH HISTORY` | Copy revision history from source branch |

### Branch Name Formats

You can use either quoted or unquoted branch names:

```sql
-- Quoted names allow special characters (slashes, hyphens)
CREATE BRANCH 'feature/new-article' FROM 'main'
CREATE BRANCH 'hotfix/urgent-fix' FROM 'production'

-- Unquoted names (alphanumeric and underscores only)
CREATE BRANCH feature_branch FROM main
CREATE BRANCH develop FROM main
```

### Revision References

The `AT REVISION` clause supports multiple formats:

```sql
-- HLC timestamp (Hybrid Logical Clock)
CREATE BRANCH 'hotfix' FROM 'main' AT REVISION 1734567890123_42

-- Git-like HEAD relative reference
CREATE BRANCH 'restore' FROM 'main' AT REVISION HEAD~5

-- Branch-relative reference
CREATE BRANCH 'restore' FROM 'main' AT REVISION develop~3
```

### Examples

```sql
-- Basic branch creation
CREATE BRANCH 'feature/new-feature' FROM 'main'

-- Create with all options
CREATE BRANCH 'develop'
  FROM 'main'
  AT REVISION HEAD~2
  DESCRIPTION 'Main development branch'
  PROTECTED
  UPSTREAM 'main'
  WITH HISTORY

-- Create orphan branch (no source)
CREATE BRANCH 'experiments'

-- Branch from specific revision
CREATE BRANCH 'hotfix/urgent' FROM 'production' AT REVISION HEAD~5
```

## DROP BRANCH

Delete a branch from the repository.

```sql
DROP BRANCH [IF EXISTS] 'branch-name'
```

### Parameters

| Parameter | Description |
|-----------|-------------|
| `IF EXISTS` | Suppress error if branch doesn't exist |
| `branch-name` | Name of the branch to delete |

### Examples

```sql
-- Delete a branch
DROP BRANCH 'feature/old-feature'

-- Delete only if exists (no error if missing)
DROP BRANCH IF EXISTS 'feature/maybe-exists'
```

**Note:** Protected branches cannot be deleted. Use `ALTER BRANCH` to remove protection first.

## ALTER BRANCH

Modify an existing branch's properties.

```sql
-- Set upstream branch
ALTER BRANCH 'branch-name' SET UPSTREAM 'upstream-branch'

-- Remove upstream tracking
ALTER BRANCH 'branch-name' UNSET UPSTREAM

-- Set/unset protected status
ALTER BRANCH 'branch-name' SET PROTECTED TRUE
ALTER BRANCH 'branch-name' SET PROTECTED FALSE

-- Update description
ALTER BRANCH 'branch-name' SET DESCRIPTION 'New description'

-- Rename branch
ALTER BRANCH 'old-name' RENAME TO 'new-name'
```

### Examples

```sql
-- Set upstream for tracking divergence
ALTER BRANCH 'feature/x' SET UPSTREAM 'main'

-- Protect production branch
ALTER BRANCH 'production' SET PROTECTED TRUE

-- Unprotect branch for deletion
ALTER BRANCH 'production' SET PROTECTED FALSE

-- Rename a branch
ALTER BRANCH 'feature/old-name' RENAME TO 'feature/new-name'

-- Remove upstream tracking
ALTER BRANCH 'feature/x' UNSET UPSTREAM

-- Add description
ALTER BRANCH 'develop' SET DESCRIPTION 'Main development branch for v2.0'
```

## MERGE BRANCH

Merge changes from one branch into another.

```sql
MERGE BRANCH 'source-branch' INTO 'target-branch'
  [USING FAST_FORWARD | THREE_WAY]
  [MESSAGE 'commit-message']
  [RESOLVE CONFLICTS (
    (node_id, KEEP_OURS | KEEP_THEIRS | DELETE | USE_VALUE 'json'),
    ...
  )]
```

### Parameters

| Parameter | Description |
|-----------|-------------|
| `source-branch` | Branch to merge from |
| `target-branch` | Branch to merge into |
| `USING` | Merge strategy (optional, defaults to THREE_WAY) |
| `MESSAGE` | Custom commit message for the merge |
| `RESOLVE CONFLICTS` | Conflict resolutions (see [Conflict Resolution](#conflict-resolution)) |

### Merge Strategies

| Strategy | Description |
|----------|-------------|
| `FAST_FORWARD` | Only succeeds if target is an ancestor of source (no merge commit) |
| `THREE_WAY` | Creates a merge commit combining both branches (default) |

### Examples

```sql
-- Basic merge
MERGE BRANCH 'feature/complete' INTO 'main'

-- Fast-forward merge (fails if not possible)
MERGE BRANCH 'hotfix/urgent' INTO 'production' USING FAST_FORWARD

-- Three-way merge with custom message
MERGE BRANCH 'feature/new-ui' INTO 'develop'
  USING THREE_WAY
  MESSAGE 'Merge new UI components into develop'

-- Merge hotfix into production
MERGE BRANCH 'hotfix/security-patch' INTO 'production'
  MESSAGE 'Apply critical security patch'

-- Merge with conflict resolutions
MERGE BRANCH 'feature/updates' INTO 'main'
  MESSAGE 'Merge updates with resolved conflicts'
  RESOLVE CONFLICTS (
    ('node-uuid-1', KEEP_OURS),
    ('node-uuid-2', KEEP_THEIRS),
    ('node-uuid-3', USE_VALUE '{"title": "Merged Title"}')
  )
```

## SHOW CONFLICTS FOR MERGE

Preview conflicts that would occur when merging two branches. Use this before attempting a merge to understand what conflicts exist.

```sql
SHOW CONFLICTS FOR MERGE 'source-branch' INTO 'target-branch'
```

### Parameters

| Parameter | Description |
|-----------|-------------|
| `source-branch` | Branch to merge from |
| `target-branch` | Branch to merge into |

### Return Columns

| Column | Description |
|--------|-------------|
| `node_id` | UUID of the conflicting node |
| `path` | Path of the conflicting node |
| `conflict_type` | Type of conflict (e.g., `BothModified`, `DeletedBySource`, `DeletedByTarget`) |
| `base_properties` | Properties at the common ancestor |
| `target_properties` | Properties on the target branch |
| `source_properties` | Properties on the source branch |
| `translation_locale` | Locale code if this is a translation conflict (optional) |

### Examples

```sql
-- Check for conflicts before merging
SHOW CONFLICTS FOR MERGE 'feature/content-updates' INTO 'main'

-- Preview conflicts for a hotfix merge
SHOW CONFLICTS FOR MERGE 'hotfix/urgent' INTO 'production'
```

## Conflict Resolution

When merging branches, conflicts can occur when:
- Both branches modified the same node (`BothModified`)
- One branch deleted a node that the other modified (`DeletedBySource` / `DeletedByTarget`)
- Both branches modified the same translation of a node

### Resolution Types

| Resolution | Description |
|------------|-------------|
| `KEEP_OURS` | Use the target branch's version |
| `KEEP_THEIRS` | Use the source branch's version |
| `DELETE` | Accept the deletion |
| `USE_VALUE 'json'` | Use a custom merged value (JSON object) |

### Resolution Syntax

Each resolution is specified as a tuple:

```sql
-- Basic resolution (node conflicts)
(node_id, RESOLUTION_TYPE)

-- Translation-aware resolution (for localized content)
(node_id, 'locale', RESOLUTION_TYPE)
```

### Examples

```sql
-- Resolve multiple conflicts with different strategies
MERGE BRANCH 'feature/redesign' INTO 'main'
  MESSAGE 'Merge redesign with conflict resolutions'
  RESOLVE CONFLICTS (
    ('uuid-article-1', KEEP_OURS),           -- Keep our version
    ('uuid-article-2', KEEP_THEIRS),         -- Accept their changes
    ('uuid-deleted-page', DELETE),            -- Accept deletion
    ('uuid-article-3', USE_VALUE '{"title": "Compromise Title", "status": "published"}')
  )

-- Resolve translation conflicts per-locale
MERGE BRANCH 'feature/translations' INTO 'main'
  MESSAGE 'Merge translations'
  RESOLVE CONFLICTS (
    ('uuid-page-1', 'en', KEEP_OURS),        -- Keep English version
    ('uuid-page-1', 'de', KEEP_THEIRS),      -- Accept German changes
    ('uuid-page-1', 'fr', USE_VALUE '{"title": "Titre fusionné"}')
  )
```

### Conflict Resolution Workflow

The recommended workflow for handling merge conflicts:

```sql
-- 1. Preview conflicts before merging
SHOW CONFLICTS FOR MERGE 'feature/updates' INTO 'main'

-- 2. Review the returned conflicts and decide on resolutions
-- (Results show node_id, path, conflict_type, and property values)

-- 3. Merge with explicit resolutions
MERGE BRANCH 'feature/updates' INTO 'main'
  MESSAGE 'Merge feature updates'
  RESOLVE CONFLICTS (
    ('conflict-node-1', KEEP_OURS),
    ('conflict-node-2', KEEP_THEIRS)
  )
```

**Note:** If you attempt a merge without providing resolutions for all conflicts, the merge will fail with an error listing the unresolved conflicts.

## Setting Branch Context

RaisinDB provides multiple ways to set the branch context for your queries. All syntax variants work across all transport layers (pgwire, HTTP SQL, WebSocket SQL).

### Session Scope

Session-scope commands set the branch for all subsequent queries in the connection (pgwire/WebSocket) or batch (HTTP).

```sql
-- RaisinDB native syntax
USE BRANCH 'branch-name'
CHECKOUT BRANCH branch-name

-- PostgreSQL-compatible syntax
SET app.branch = 'branch-name'
SET app.branch TO 'branch-name'
```

### Statement Scope

Statement-scope commands set the branch for only the next query, then revert to the session branch.

```sql
-- RaisinDB native syntax
USE LOCAL BRANCH 'branch-name'

-- PostgreSQL-compatible syntax
SET LOCAL app.branch = 'branch-name'
SET LOCAL app.branch TO 'branch-name'
```

### Examples

```sql
-- Switch to develop branch for the session
USE BRANCH 'develop'

-- Now all queries use the develop branch
SELECT * FROM content WHERE node_type = 'cms:Article'

-- PostgreSQL-compatible way to switch branches
SET app.branch = 'feature/new-ui'

-- Execute one query on production, then revert to session branch
USE LOCAL BRANCH 'production'
SELECT COUNT(*) FROM content  -- Uses 'production'
SELECT * FROM content LIMIT 10  -- Back to 'feature/new-ui'

-- Using SET LOCAL for a single query
SET LOCAL app.branch = 'staging'
SELECT * FROM content WHERE path = '/home'  -- Uses 'staging'
-- Subsequent queries use session branch again
```

## SHOW Statements

### SHOW BRANCHES

List all branches in the repository.

```sql
SHOW BRANCHES
```

Returns a list of all branches with their metadata.

### SHOW CURRENT BRANCH

Display the current session branch.

```sql
-- RaisinDB native syntax
SHOW CURRENT BRANCH

-- PostgreSQL-compatible alias
SHOW app.branch
```

Both commands return the effective branch for the current session.

### DESCRIBE BRANCH

Show detailed information about a specific branch.

```sql
DESCRIBE BRANCH 'branch-name'
```

Returns branch details including:
- Name
- Head revision
- Created from (source branch)
- Upstream branch
- Protected status
- Creation date

### SHOW DIVERGENCE

Display how many commits a branch is ahead or behind another.

```sql
SHOW DIVERGENCE 'branch' FROM 'base'
```

### Examples

```sql
-- List all branches
SHOW BRANCHES

-- Check current branch
SHOW CURRENT BRANCH

-- Get branch details
DESCRIBE BRANCH 'main'
DESCRIBE BRANCH 'feature/new-feature'

-- Check divergence from main
SHOW DIVERGENCE 'feature/x' FROM 'main'
SHOW DIVERGENCE 'develop' FROM 'production'
```

## Branch Context Priority

When multiple branch specifications are present, RaisinDB uses the following priority order (highest to lowest):

| Priority | Source | Scope |
|----------|--------|-------|
| 1 | `USE LOCAL BRANCH` / `SET LOCAL app.branch` | Single statement |
| 2 | Request context branch (WebSocket) | Per-request |
| 3 | `USE BRANCH` / `SET app.branch` | Session |
| 4 | URL path branch (HTTP `/api/sql/{repo}/{branch}`) | Request |
| 5 | Repository's configured `default_branch` | Default |

### Example

```sql
-- Repository default_branch is 'main'
-- Session has: SET app.branch = 'develop'

SELECT * FROM content;  -- Uses 'develop' (session)

USE LOCAL BRANCH 'staging';
SELECT * FROM content;  -- Uses 'staging' (local override)

SELECT * FROM content;  -- Uses 'develop' (back to session)
```

## HTTP SQL Endpoints

RaisinDB provides two HTTP endpoints for SQL queries:

### Default Branch Endpoint

```
POST /api/sql/{repository}
```

Uses the repository's configured `default_branch`. Supports `USE BRANCH` within the SQL batch.

### Explicit Branch Endpoint

```
POST /api/sql/{repository}/{branch}
```

Uses the branch specified in the URL path. `USE BRANCH` within the SQL batch can still override for subsequent statements.

### Example Requests

```bash
# Use repository's default branch
curl -X POST "http://localhost:8080/api/sql/my-repo" \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT * FROM content LIMIT 10"}'

# Use explicit branch in URL
curl -X POST "http://localhost:8080/api/sql/my-repo/develop" \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT * FROM content LIMIT 10"}'

# Switch branches within a batch
curl -X POST "http://localhost:8080/api/sql/my-repo" \
  -H "Content-Type: application/json" \
  -d '{"sql": "USE BRANCH '\''staging'\''; SELECT COUNT(*) FROM content"}'
```

## Common Workflows

### Feature Branch Workflow

```sql
-- 1. Create a feature branch
CREATE BRANCH 'feature/user-profiles' FROM 'main'
  DESCRIPTION 'Add user profile management'
  UPSTREAM 'main'

-- 2. Switch to the feature branch
USE BRANCH 'feature/user-profiles'

-- 3. Make changes (INSERT, UPDATE, etc.)
-- ... work on the feature ...

-- 4. Check divergence before merging
SHOW DIVERGENCE 'feature/user-profiles' FROM 'main'

-- 5. Merge back to main
MERGE BRANCH 'feature/user-profiles' INTO 'main'
  MESSAGE 'Add user profile management feature'

-- 6. Clean up feature branch
DROP BRANCH 'feature/user-profiles'
```

### Release Branch Workflow

```sql
-- 1. Create release branch from develop
CREATE BRANCH 'release/v2.0' FROM 'develop'
  PROTECTED
  DESCRIPTION 'Release candidate for version 2.0'

-- 2. Apply hotfixes if needed
CREATE BRANCH 'hotfix/release-fix' FROM 'release/v2.0'
-- ... fix issues ...
MERGE BRANCH 'hotfix/release-fix' INTO 'release/v2.0'

-- 3. Merge to production
MERGE BRANCH 'release/v2.0' INTO 'production'
  USING THREE_WAY
  MESSAGE 'Release v2.0'

-- 4. Merge back to develop
MERGE BRANCH 'release/v2.0' INTO 'develop'
```

### Content Staging Workflow

```sql
-- 1. Create staging branch for content preview
CREATE BRANCH 'staging/winter-campaign' FROM 'production'
  DESCRIPTION 'Winter marketing campaign content'

-- 2. Switch to staging
USE BRANCH 'staging/winter-campaign'

-- 3. Add/edit content (visible only in staging)
-- ... edit content ...

-- 4. Preview and review content
SELECT * FROM content WHERE PATH_STARTS_WITH(path, '/campaigns/winter/')

-- 5. When approved, merge to production
MERGE BRANCH 'staging/winter-campaign' INTO 'production'
  MESSAGE 'Publish winter campaign content'

-- 6. Clean up
DROP BRANCH 'staging/winter-campaign'
```

## Querying with Branches

When querying data, you can filter by branch using the virtual `__branch` column:

```sql
-- Query specific branch data
SELECT id, name, path, __branch
FROM content
WHERE __branch = 'develop'

-- Compare data across branches
SELECT
  m.path,
  m.properties ->> 'title' AS main_title,
  d.properties ->> 'title' AS develop_title
FROM (SELECT * FROM content WHERE __branch = 'main') m
JOIN (SELECT * FROM content WHERE __branch = 'develop') d ON d.path = m.path
WHERE m.properties ->> 'title' != d.properties ->> 'title'
```

## Quick Reference

| Statement | Description |
|-----------|-------------|
| `CREATE BRANCH 'x' FROM 'y'` | Create new branch |
| `DROP BRANCH 'x'` | Delete branch |
| `DROP BRANCH IF EXISTS 'x'` | Delete if exists |
| `ALTER BRANCH 'x' SET UPSTREAM 'y'` | Set upstream |
| `ALTER BRANCH 'x' UNSET UPSTREAM` | Remove upstream |
| `ALTER BRANCH 'x' SET PROTECTED TRUE` | Protect branch |
| `ALTER BRANCH 'x' RENAME TO 'y'` | Rename branch |
| `MERGE BRANCH 'x' INTO 'y'` | Merge branches |
| `MERGE BRANCH ... RESOLVE CONFLICTS (...)` | Merge with conflict resolutions |
| `SHOW CONFLICTS FOR MERGE 'x' INTO 'y'` | Preview merge conflicts |
| `USE BRANCH 'x'` | Set session branch |
| `CHECKOUT BRANCH x` | Set session branch (alias) |
| `SET app.branch = 'x'` | Set session branch (PostgreSQL-compatible) |
| `USE LOCAL BRANCH 'x'` | Set branch for next query only |
| `SET LOCAL app.branch = 'x'` | Set branch for next query only (PostgreSQL-compatible) |
| `SHOW BRANCHES` | List all branches |
| `SHOW CURRENT BRANCH` | Show session branch |
| `SHOW app.branch` | Show session branch (PostgreSQL-compatible) |
| `DESCRIBE BRANCH 'x'` | Show branch details |
| `SHOW DIVERGENCE 'x' FROM 'y'` | Show commits ahead/behind |

## What's Next?

- [RaisinSQL Reference](raisinsql.md) - Full SQL reference
- [Query Examples](examples.md) - Real-world query patterns
- [Cypher Graph Queries](cypher.md) - Graph pattern matching
