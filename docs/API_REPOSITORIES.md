# Repository Management API

This document describes the HTTP API endpoints for managing repositories in RaisinDB.

## Overview

Repositories are the top-level containers for content in RaisinDB's repository-first architecture. Each repository contains:
- Branches (like git branches)
- Tags (immutable markers at specific revisions)
- Workspaces (content containers within a branch)
- Revisions (commit history)

## Endpoints

### List Repositories

List all repositories for the current tenant.

**Request:**
```http
GET /api/repositories
X-Tenant-ID: {tenant_id}
```

**Response:** `200 OK`
```json
[
  {
    "tenant_id": "acme-corp",
    "repo_id": "website",
    "created_at": "2025-01-15T10:30:00Z",
    "branches": ["main", "develop"],
    "config": {
      "default_branch": "main",
      "description": "Corporate website content",
      "tags": {}
    }
  }
]
```

### Get Repository

Get information about a specific repository.

**Request:**
```http
GET /api/repositories/{repo_id}
X-Tenant-ID: {tenant_id}
```

**Response:** `200 OK`
```json
{
  "tenant_id": "acme-corp",
  "repo_id": "website",
  "created_at": "2025-01-15T10:30:00Z",
  "branches": ["main", "develop", "feature/new-design"],
  "config": {
    "default_branch": "main",
    "description": "Corporate website content",
    "tags": {
      "environment": "production"
    }
  }
}
```

**Error Responses:**
- `404 Not Found` - Repository does not exist

### Create Repository

Create a new repository for the tenant.

**Request:**
```http
POST /api/repositories
X-Tenant-ID: {tenant_id}
Content-Type: application/json

{
  "repo_id": "website",
  "description": "Corporate website content",
  "default_branch": "main"
}
```

**Parameters:**
- `repo_id` (required) - Unique identifier for the repository (e.g., "website", "blog")
- `description` (optional) - Human-readable description
- `default_branch` (optional) - Default branch name, defaults to "main"

**Response:** `201 Created`
```json
{
  "tenant_id": "acme-corp",
  "repo_id": "website",
  "created_at": "2025-01-15T10:30:00Z",
  "branches": [],
  "config": {
    "default_branch": "main",
    "description": "Corporate website content",
    "tags": {}
  }
}
```

**Error Responses:**
- `409 Conflict` - Repository already exists
- `400 Bad Request` - Invalid repository ID or configuration

### Update Repository

Update repository configuration.

**Request:**
```http
PUT /api/repositories/{repo_id}
X-Tenant-ID: {tenant_id}
Content-Type: application/json

{
  "description": "Updated description",
  "default_branch": "production"
}
```

**Parameters:**
- `description` (optional) - New description
- `default_branch` (optional) - New default branch name

**Response:** `204 No Content`

**Error Responses:**
- `404 Not Found` - Repository does not exist
- `400 Bad Request` - Invalid configuration

### Delete Repository

Delete a repository and all its content.

**WARNING:** This permanently deletes:
- All branches
- All tags
- All revisions
- All workspaces
- All nodes and content

This operation cannot be undone.

**Request:**
```http
DELETE /api/repositories/{repo_id}
X-Tenant-ID: {tenant_id}
```

**Response:** `204 No Content`

**Error Responses:**
- `404 Not Found` - Repository does not exist

## Common Workflows

### Initial Repository Setup

```bash
# 1. Create repository
curl -X POST http://localhost:8080/api/repositories \
  -H "X-Tenant-ID: acme-corp" \
  -H "Content-Type: application/json" \
  -d '{
    "repo_id": "website",
    "description": "Corporate website",
    "default_branch": "main"
  }'

# 2. Create main branch
curl -X POST http://localhost:8080/api/management/repositories/acme-corp/website/branches \
  -H "Content-Type: application/json" \
  -d '{
    "branch_name": "main",
    "created_by": "admin",
    "from_revision": null,
    "protected": true
  }'

# 3. Create development branch
curl -X POST http://localhost:8080/api/management/repositories/acme-corp/website/branches \
  -H "Content-Type: application/json" \
  -d '{
    "branch_name": "develop",
    "created_by": "admin",
    "from_revision": null,
    "protected": false
  }'

# 4. Start working with content
curl -X POST http://localhost:8080/api/repository/website/main/demo/ \
  -H "Content-Type: application/json" \
  -d '{
    "name": "homepage",
    "node_type": "raisin:Page",
    "properties": {
      "title": "Welcome"
    }
  }'
```

### Migration from Old Structure

If you have existing content using the old URL structure without repository/branch:

```bash
# Old URL (deprecated):
# /api/repository/{workspace}/path

# New URL (repository-first):
# /api/repository/{repo}/{branch}/{workspace}/path

# Example migration:
# OLD: GET /api/repository/demo/content/homepage
# NEW: GET /api/repository/default/main/demo/content/homepage
```

## Multi-Tenant Considerations

In single-tenant mode (default):
- `X-Tenant-ID` header is optional, defaults to "default"
- Repository IDs must be unique within the tenant

In multi-tenant mode:
- `X-Tenant-ID` header is required
- Repository IDs must be unique within each tenant
- Different tenants can have repositories with the same ID

## See Also

- [Branch Management API](./API_BRANCHES_TAGS.md#branches) - Managing branches within a repository
- [Tag Management API](./API_BRANCHES_TAGS.md#tags) - Managing tags within a repository  
- [Node Operations](./API_QUICK_REFERENCE.md) - Working with content nodes
- [Versioning & Publishing](./API_DESIGN.md#versioning) - Version control and publishing workflows
