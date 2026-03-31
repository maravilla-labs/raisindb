# RaisinDB Transport HTTP - REST API Reference

This document describes the repository HTTP handlers provided by `raisin-transport-http`.

## Quick Reference: Upload Patterns

| Mode | Endpoint | Use Case |
|------|----------|----------|
| **One-Shot Upload** | `POST /api/repository/{repo}/{branch}/head/{ws}/{path}` | Auto-create `raisin:Asset` with file |
| **Upload to Property** | `POST .../{path}@properties.{prop}` | Upload to specific property on existing node |
| **Package Upload** | `POST /api/repos/{repo}/packages/upload` | Upload `.rap` package file |

### Query Parameters (File Upload)

| Param | Default | Description |
|-------|---------|-------------|
| `node_type` | `raisin:Asset` | NodeType for auto-created node |
| `property_path` | `file` | Property to store Resource |
| `inline` | `false` | Store as UTF-8 string (max 11MB) |
| `override_existing` | `false` | Replace existing file |
| `commit_message` | - | Optional: create revision if provided |
| `commit_actor` | `system` | Actor for commit |

## URL Structure

```
/api/repository/{repo}/{branch}/{head|rev/{revision}}/{workspace}/*path
```

### Path Components

| Component | Description | Example |
|-----------|-------------|---------|
| `repo` | Repository identifier | `website` |
| `branch` | Branch name | `main`, `feature-x` |
| `head` or `rev/{revision}` | Head (latest) or specific revision | `head`, `rev/123456` |
| `workspace` | Workspace name | `content`, `assets` |
| `*path` | Node path within workspace | `/blog/my-post` |

### Path Modifiers

#### Property Path (`@property`)

Access a specific property within a node:

```
/api/repository/myrepo/main/head/content/blog/post@properties.title
/api/repository/myrepo/main/head/content/blog/post@file
```

#### Command Pattern (`raisin:cmd/{command}`)

Execute a command on a node:

```
/api/repository/myrepo/main/head/content/blog/post/raisin:cmd/download
/api/repository/myrepo/main/head/content/blog/post/raisin:cmd/relations
```

#### Version Pattern (`raisin:version/{id}`)

Access node version history:

```
/api/repository/myrepo/main/head/content/blog/post/raisin:version
/api/repository/myrepo/main/head/content/blog/post/raisin:version/5
```

#### Combined Patterns

Property path can be combined with command pattern:

```
/api/repository/myrepo/main/head/content/blog/post@properties.attachment/raisin:cmd/download
```

## File Upload

### Auto-Create Behavior

When uploading to a path where no node exists, a new `raisin:Asset` node is automatically created with:
- `name`: Derived from filename or path
- `title`: Set to the filename
- `node_type`: `raisin:Asset`
- `file`: The uploaded Resource

This allows single-step file uploads without needing to create the node first.

### External Storage Upload (Default)

Files are streamed to binary storage without buffering the entire body in memory.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/*path
Content-Type: multipart/form-data

<file data>
```

**Query Parameters:**
- `override_existing=true` - Replace existing file (deletes old file from storage)
- `commit_message={message}` - Custom commit message for transaction
- `commit_actor={actor}` - Actor name for audit trail

**Response:**
```json
{"storedKey": "2025/01/15/abc123.png", "url": "/files/2025/01/15/abc123.png"}
```

**Stored Property Structure:**
```json
{
  "file": {
    "uuid": "abc123xyz",
    "name": "document.pdf",
    "size": 12345,
    "mime_type": "application/pdf",
    "url": "2025/01/15/abc123xyz.pdf",
    "metadata": {
      "storage_key": "2025/01/15/abc123xyz.pdf"
    },
    "is_loaded": true,
    "is_external": false,
    "created_at": "2025-01-15T10:30:00Z",
    "updated_at": "2025-01-15T10:30:00Z"
  }
}
```

### Inline Upload

Small files (max 11MB) can be stored directly as string content in the node properties.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/*path?inline=true
Content-Type: multipart/form-data

<file data>
```

**Requirements:**
- File content must be valid UTF-8
- Maximum size: 11MB

**Stored Property Structure:**
```json
{
  "file": "file content as string...",
  "file_type": "text/plain",
  "file_size": 1234
}
```

### Upload to Specific Property

Use the `@property` notation to upload to a specific property:

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/blog/post@properties.attachment
Content-Type: multipart/form-data

<file data>
```

## File Download

### Download Command

Download a file as an attachment with `Content-Disposition: attachment` header.

**Using command pattern (preferred):**
```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path/raisin:cmd/download
```

**Using query parameter:**
```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path?command=download
```

**Download specific property:**
```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@properties.attachment/raisin:cmd/download
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@properties.attachment?command=download
```

**Response:**
- Binary file content
- `Content-Type`: Detected from stored MIME type or guessed from filename
- `Content-Disposition: attachment; filename="original-filename.ext"`

### Auto-Detect Streaming (Inline Display)

When accessing a property that contains a Resource or file-like String, the content is streamed inline without attachment headers.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@properties.file
```

**Behavior by PropertyValue type:**

| Type | Behavior |
|------|----------|
| `Resource` (internal) | Stream binary content with `Content-Type` header |
| `Resource` (external) | 307 redirect to external URL |
| `String` (file-like) | Return as text with guessed `Content-Type` |
| Other types | Return as JSON |

**Example - Display image inline:**
```bash
# Returns image binary with Content-Type: image/png
curl http://localhost:8080/api/repository/myrepo/main/head/assets/logo@file
```

**Example - Download image as attachment:**
```bash
# Returns image with Content-Disposition: attachment
curl http://localhost:8080/api/repository/myrepo/main/head/assets/logo/raisin:cmd/download
```

## PropertyValue::Resource Structure

```rust
pub struct Resource {
    pub uuid: String,                              // Unique identifier (nanoid)
    pub name: Option<String>,                      // Original filename
    pub size: Option<i64>,                         // File size in bytes
    pub mime_type: Option<String>,                 // Content type
    pub url: Option<String>,                       // Storage key or external URL
    pub metadata: Option<HashMap<String, PropertyValue>>,
    pub is_loaded: Option<bool>,                   // Whether content is loaded
    pub is_external: Option<bool>,                 // External URL vs internal storage
    pub created_at: DateTimeTimestamp,
    pub updated_at: DateTimeTimestamp,
}
```

**Key Fields:**

| Field | Internal Storage | External URL |
|-------|------------------|--------------|
| `is_external` | `false` | `true` |
| `url` | Storage key (e.g., `2025/01/15/abc.pdf`) | Full URL |
| `metadata.storage_key` | Same as `url` | N/A |

## GET Commands

Commands available via `raisin:cmd/{command}` pattern:

| Command | Description |
|---------|-------------|
| `download` | Download file as attachment |
| `relations` | Get node relationships |
| `list-translations` | List available translation locales |

## POST Commands

Commands available via POST `raisin:cmd/{command}` or `?command={command}`:

| Command | Description | Body |
|---------|-------------|------|
| `rename` | Rename node | `{"newName": "new-name"}` |
| `move` | Move node | `{"targetPath": "/new/location"}` |
| `copy` | Copy single node | `{"targetPath": "/dest", "newName": "name"}` |
| `copy_tree` | Copy with descendants | `{"targetPath": "/dest", "newName": "name"}` |
| `publish` | Publish node | `{}` |
| `publish_tree` | Publish with descendants | `{}` |
| `unpublish` | Unpublish node | `{}` |
| `unpublish_tree` | Unpublish with descendants | `{}` |
| `reorder` | Reorder among siblings | `{"targetPath": "/sibling", "movePosition": "before\|after"}` |

## Middleware: RaisinContext

The `raisin_parsing_middleware` parses incoming requests and extracts:

```rust
pub struct RaisinContext {
    pub repo_name: String,
    pub branch_name: String,
    pub workspace_name: String,
    pub cleaned_path: String,        // Path without modifiers
    pub original_path: String,
    pub file_extension: Option<String>,
    pub is_version: bool,
    pub version_id: Option<i32>,
    pub is_command: bool,
    pub command_name: Option<String>,
    pub property_path: Option<String>,
    pub archetype: String,           // Content-Type header
}
```

## Binary Storage

Files are stored in configurable binary storage backends:

### Filesystem (Default)

```rust
FilesystemBinaryStorage::new("./.data/uploads", "/files")
```

Key format: `{YYYY}/{MM}/{DD}/{nanoid}.{ext}`

### S3/R2 (with `s3` feature)

Configured via environment variables:
- `R2_BUCKET`
- `R2_ACCESS_KEY_ID`
- `R2_SECRET_ACCESS_KEY`
- `R2_ENDPOINT`

## Error Responses

| Status | Code | Description |
|--------|------|-------------|
| 400 | `VALIDATION_ERROR` | Invalid request (e.g., unknown command) |
| 404 | `NOT_FOUND` | Node or property not found |
| 500 | `STORAGE_ERROR` | Binary storage retrieval failed |

## Examples

### Upload and Display Image

```bash
# Upload image
curl -X POST http://localhost:8080/api/repository/myrepo/main/head/assets/logo \
  -F "file=@logo.png"

# Display inline (browser shows image)
curl http://localhost:8080/api/repository/myrepo/main/head/assets/logo@file

# Download as file
curl http://localhost:8080/api/repository/myrepo/main/head/assets/logo/raisin:cmd/download
```

### Upload to Specific Property

```bash
# Upload to properties.thumbnail
curl -X POST "http://localhost:8080/api/repository/myrepo/main/head/content/post@properties.thumbnail" \
  -F "file=@thumb.jpg"

# Access the thumbnail inline
curl http://localhost:8080/api/repository/myrepo/main/head/content/post@properties.thumbnail
```

### Inline Upload for Code/Text

```bash
# Upload code file inline (stored as string)
curl -X POST "http://localhost:8080/api/repository/myrepo/main/head/code/script?inline=true" \
  -F "file=@script.js"

# Read back the code
curl http://localhost:8080/api/repository/myrepo/main/head/code/script@file
```

## Package API

Packages are `.rap` files (ZIP archives) containing node types, workspaces, and content that can be installed into a repository.

### Package Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/repos/{repo}/packages` | List all packages |
| `GET` | `/api/repos/{repo}/packages/{name}` | Get package details |
| `POST` | `/api/repos/{repo}/packages/upload` | Upload a `.rap` package |
| `POST` | `/api/repos/{repo}/packages/{name}/install` | Install package (async job) |
| `POST` | `/api/repos/{repo}/packages/{name}/uninstall` | Uninstall package |
| `GET` | `/api/repos/{repo}/packages/{name}/contents` | List ZIP contents |
| `GET` | `/api/repos/{repo}/packages/{name}/contents/{path}` | Get file from ZIP |

### Upload Package

```bash
curl -X POST "http://localhost:8080/api/repos/default/packages/upload" \
  -H "Authorization: Bearer $TOKEN" \
  -F "file=@mypackage-1.0.0.rap"
```

**Response:**
```json
{
  "package_name": "mypackage",
  "version": "1.0.0",
  "node_id": "package-mypackage"
}
```

### Install Package

```bash
curl -X POST "http://localhost:8080/api/repos/default/packages/mypackage/install" \
  -H "Authorization: Bearer $TOKEN"
```

**Response:**
```json
{
  "package_name": "mypackage",
  "version": "1.0.0",
  "installed": false,
  "job_id": "job-abc123"
}
```

Installation is asynchronous via the job system. Monitor progress via the jobs API.

### CLI Usage

```bash
# Create a package from a folder
raisindb package create ./my-package -o mypackage-1.0.0.rap

# Upload package to server
raisindb package upload mypackage-1.0.0.rap -s http://localhost:8081 -r default

# List packages
raisindb package list -s http://localhost:8081 -r default

# Install a package
raisindb package install mypackage -s http://localhost:8081 -r default
```

### Package Manifest (manifest.yaml)

```yaml
name: mypackage
version: 1.0.0
title: My Package
description: A sample package
author: Your Name
category: content
keywords: [sample, demo]
icon: 📦

provides:
  nodetypes:
    - myns:CustomType
  workspaces:
    - mycontent
  content:
    - /mycontent/default-data

dependencies:
  - name: base-package
    version: ">=1.0.0"

workspace_patches:
  content:
    allowed_node_types:
      add:
        - myns:CustomType
```
