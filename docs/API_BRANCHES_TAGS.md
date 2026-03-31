# Branch and Tag Management API

## Overview

RaisinDB provides REST API endpoints for managing Git-like branches and tags at the repository level. These endpoints enable version control workflows including branch creation, tagging releases, and managing repository history.

## Base URL Pattern

All branch and tag endpoints follow the pattern:
```
/api/management/repositories/{tenant_id}/{repo_id}/...
```

- `{tenant_id}`: The tenant identifier (use "default" for single-tenant deployments)
- `{repo_id}`: The repository identifier (e.g., "main", "website", "mobile-app")

## Branch Management

### Create Branch

Create a new branch in a repository.

**Endpoint:** `POST /api/management/repositories/{tenant_id}/{repo_id}/branches`

**Request Body:**
```json
{
  "name": "develop",
  "from_revision": 42,
  "created_by": "user@example.com",
  "protected": false
}
```

**Fields:**
- `name` (required): Name for the new branch
- `from_revision` (optional): Revision number to branch from. If null, creates from scratch (revision 0)
- `created_by` (optional): Actor creating the branch. Defaults to "system"
- `protected` (optional): Whether the branch is protected from deletion. Defaults to false

**Response:** `201 Created`
```json
{
  "name": "develop",
  "head": 42,
  "created_at": "2025-10-14T10:30:00Z",
  "created_by": "user@example.com",
  "created_from": 42,
  "protected": false
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/management/repositories/default/main/branches \
  -H "Content-Type: application/json" \
  -d '{
    "name": "develop",
    "from_revision": null,
    "created_by": "devops@example.com",
    "protected": false
  }'
```

---

### List Branches

Get all branches in a repository.

**Endpoint:** `GET /api/management/repositories/{tenant_id}/{repo_id}/branches`

**Response:** `200 OK`
```json
[
  {
    "name": "main",
    "head": 100,
    "created_at": "2025-01-01T00:00:00Z",
    "created_by": "system",
    "created_from": null,
    "protected": true
  },
  {
    "name": "develop",
    "head": 95,
    "created_at": "2025-10-01T00:00:00Z",
    "created_by": "dev-team",
    "created_from": 90,
    "protected": false
  }
]
```

**Example:**
```bash
curl http://localhost:8080/api/management/repositories/default/main/branches
```

---

### Get Branch

Get information about a specific branch.

**Endpoint:** `GET /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}`

**Response:** `200 OK`
```json
{
  "name": "develop",
  "head": 95,
  "created_at": "2025-10-01T00:00:00Z",
  "created_by": "dev-team",
  "created_from": 90,
  "protected": false
}
```

**Error Response:** `404 Not Found` if branch doesn't exist

**Example:**
```bash
curl http://localhost:8080/api/management/repositories/default/main/branches/develop
```

---

### Delete Branch

Delete a branch from a repository.

**Endpoint:** `DELETE /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}`

**Response:** 
- `204 No Content` on success
- `404 Not Found` if branch doesn't exist
- `403 Forbidden` if branch is protected (implementation-dependent)

**Example:**
```bash
curl -X DELETE http://localhost:8080/api/management/repositories/default/main/branches/temp-branch
```

---

### Get Branch HEAD

Get the current HEAD revision for a branch.

**Endpoint:** `GET /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head`

**Response:** `200 OK`
```json
95
```

**Example:**
```bash
curl http://localhost:8080/api/management/repositories/default/main/branches/develop/head
```

---

### Update Branch HEAD

Update the HEAD pointer for a branch (advance or rewind).

**Endpoint:** `PUT /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head`

**Request Body:**
```json
{
  "revision": 100
}
```

**Response:** `204 No Content`

**Example:**
```bash
curl -X PUT http://localhost:8080/api/management/repositories/default/main/branches/develop/head \
  -H "Content-Type: application/json" \
  -d '{"revision": 100}'
```

---

## Tag Management

### Create Tag

Create a new tag pointing to a specific revision.

**Endpoint:** `POST /api/management/repositories/{tenant_id}/{repo_id}/tags`

**Request Body:**
```json
{
  "name": "v1.0.0",
  "revision": 100,
  "created_by": "release-manager@example.com",
  "message": "First stable release",
  "protected": true
}
```

**Fields:**
- `name` (required): Tag name (e.g., "v1.0.0", "release-2024-10-14")
- `revision` (required): Revision number this tag points to
- `created_by` (optional): Actor who created the tag. Defaults to "system"
- `message` (optional): Annotation message describing the tag
- `protected` (optional): Whether the tag is protected from deletion. Defaults to false

**Response:** `201 Created`
```json
{
  "name": "v1.0.0",
  "revision": 100,
  "created_at": "2025-10-14T10:30:00Z",
  "created_by": "release-manager@example.com",
  "message": "First stable release",
  "protected": true
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/management/repositories/default/main/tags \
  -H "Content-Type: application/json" \
  -d '{
    "name": "v1.0.0",
    "revision": 100,
    "created_by": "release@example.com",
    "message": "Production release v1.0.0",
    "protected": true
  }'
```

---

### List Tags

Get all tags in a repository.

**Endpoint:** `GET /api/management/repositories/{tenant_id}/{repo_id}/tags`

**Response:** `200 OK`
```json
[
  {
    "name": "v1.0.0",
    "revision": 100,
    "created_at": "2025-10-01T00:00:00Z",
    "created_by": "release-manager",
    "message": "First stable release",
    "protected": true
  },
  {
    "name": "v1.1.0",
    "revision": 150,
    "created_at": "2025-10-14T00:00:00Z",
    "created_by": "release-manager",
    "message": "Feature update",
    "protected": true
  }
]
```

**Example:**
```bash
curl http://localhost:8080/api/management/repositories/default/main/tags
```

---

### Get Tag

Get information about a specific tag.

**Endpoint:** `GET /api/management/repositories/{tenant_id}/{repo_id}/tags/{name}`

**Response:** `200 OK`
```json
{
  "name": "v1.0.0",
  "revision": 100,
  "created_at": "2025-10-01T00:00:00Z",
  "created_by": "release-manager",
  "message": "First stable release",
  "protected": true
}
```

**Error Response:** `404 Not Found` if tag doesn't exist

**Example:**
```bash
curl http://localhost:8080/api/management/repositories/default/main/tags/v1.0.0
```

---

### Delete Tag

Delete a tag from a repository.

**Endpoint:** `DELETE /api/management/repositories/{tenant_id}/{repo_id}/tags/{name}`

**Response:** 
- `204 No Content` on success
- `404 Not Found` if tag doesn't exist
- `403 Forbidden` if tag is protected (implementation-dependent)

**Example:**
```bash
curl -X DELETE http://localhost:8080/api/management/repositories/default/main/tags/temp-snapshot
```

---

## Common Workflows

### Creating a Feature Branch

```bash
# 1. Get current HEAD of main branch
MAIN_HEAD=$(curl -s http://localhost:8080/api/management/repositories/default/main/branches/main/head)

# 2. Create feature branch from main's HEAD
curl -X POST http://localhost:8080/api/management/repositories/default/main/branches \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"feature/new-feature\",
    \"from_revision\": $MAIN_HEAD,
    \"created_by\": \"developer@example.com\",
    \"protected\": false
  }"
```

### Tagging a Release

```bash
# 1. Get current HEAD of production branch
PROD_HEAD=$(curl -s http://localhost:8080/api/management/repositories/default/main/branches/production/head)

# 2. Create release tag
curl -X POST http://localhost:8080/api/management/repositories/default/main/tags \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"release-$(date +%Y-%m-%d)\",
    \"revision\": $PROD_HEAD,
    \"created_by\": \"ci-cd@example.com\",
    \"message\": \"Production deployment $(date)\",
    \"protected\": true
  }"
```

### Listing All Branches and Tags

```bash
# List all branches
echo "=== Branches ==="
curl -s http://localhost:8080/api/management/repositories/default/main/branches | jq '.[] | {name, head, protected}'

# List all tags
echo "\n=== Tags ==="
curl -s http://localhost:8080/api/management/repositories/default/main/tags | jq '.[] | {name, revision, message}'
```

---

## Error Codes

| Status Code | Description |
|-------------|-------------|
| `200 OK` | Successful GET request |
| `201 Created` | Resource successfully created |
| `204 No Content` | Successful DELETE or PUT with no response body |
| `404 Not Found` | Branch or tag doesn't exist |
| `409 Conflict` | Branch or tag already exists (on creation) |
| `500 Internal Server Error` | Server-side error |

---

## Notes

- Branch names must be unique within a repository
- Tag names must be unique within a repository
- Protected branches/tags cannot be deleted (protection enforcement depends on implementation)
- Revision numbers are monotonically increasing integers starting from 0
- All timestamps are in ISO 8601 format (UTC)
