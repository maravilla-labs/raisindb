# REST API Reference

Complete reference for the RaisinDB REST API provided by `raisin-server`.

## Overview

The `raisin-server` binary provides a complete HTTP REST API for interacting with RaisinDB. It exposes:

- **Repository-style routes** for path-based node operations
- **NodeType management** for schema definition
- **Query endpoints** for filtering and searching
- **Command execution** for complex operations
- **File upload/download** for binary assets
- **Workspace management** for configuration
- **Audit logging** for change tracking

## Server Setup

### Running the Server

```bash
# Clone the repository
git clone https://github.com/yourusername/raisindb
cd raisindb

# Build with RocksDB storage, WebSocket, and PGWire (default features)
cargo build --release --package raisin-server --features "storage-rocksdb,websocket,pgwire"

# Run with a configuration file
RUST_LOG=info ./target/release/raisin-server --config examples/cluster/node1.toml
```

### Configuration

The server can be configured via feature flags:

```bash
# Use RocksDB storage (persistent, default)
cargo build --release --package raisin-server --features storage-rocksdb

# Use S3 for binary storage
cargo build --release --package raisin-server --features "storage-rocksdb,s3"
```

### Base URL

```
http://localhost:8080
```

All endpoints are prefixed with the base URL.

## URL Structure

Most API endpoints follow a repository-first URL pattern:

```
/api/repository/{repo}/{branch}/head/{workspace}/...
```

- `{repo}` - Repository identifier (project/database)
- `{branch}` - Branch name (e.g., `main`)
- `head` - Indicates current state (use `rev/{revision}` for historical snapshots)
- `{workspace}` - Workspace name

For example, to access nodes in the `content` workspace on the `main` branch of the `myapp` repository:

```
/api/repository/myapp/main/head/content/
```

## Authentication

Authentication is applied automatically via middleware when using the RocksDB storage backend (`storage-rocksdb` feature). The server uses optional authentication - requests with valid credentials get an authenticated context, while unauthenticated requests proceed with limited access.

## Repository Routes

Path-based CRUD operations for nodes. All repository routes follow the pattern:

```
/api/repository/{repo}/{branch}/head/{workspace}/...
```

In the examples below, we use `myapp` as the repo, `main` as the branch, and `content` as the workspace.

### List Root Nodes

Get all nodes at the root level of a workspace.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/
```

**Example:**
```bash
curl http://localhost:8080/api/repository/myapp/main/head/content/
```

**Response:**
```json
[
  {
    "id": "abc123",
    "name": "homepage",
    "path": "/homepage",
    "node_type": "myapp:Page",
    "properties": {...},
    "children": [],
    "version": 1,
    "created_at": "2024-01-01T00:00:00Z"
  }
]
```

### Create Node at Root

Create a new node at the root of a workspace.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/
Content-Type: application/json

{
  "name": "my-page",
  "node_type": "myapp:Page",
  "properties": {
    "title": "My Page"
  }
}
```

**Query Parameters:**
- `deep=true` - Auto-create missing parent folders

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/ \
  -H "Content-Type: application/json" \
  -d '{
    "name": "homepage",
    "node_type": "myapp:Page",
    "properties": {
      "title": "Welcome"
    }
  }'
```

**Response:**
```json
{
  "id": "abc123",
  "name": "homepage",
  "path": "/homepage",
  "node_type": "myapp:Page",
  "properties": {
    "title": "Welcome"
  },
  "version": 1,
  "created_at": "2024-01-01T00:00:00Z"
}
```

### Get Node by ID

Get a node by its unique ID.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/$ref/{id}
```

**Example:**
```bash
curl http://localhost:8080/api/repository/myapp/main/head/content/\$ref/abc123
```

**Response:**
```json
{
  "id": "abc123",
  "name": "homepage",
  "path": "/homepage",
  "node_type": "myapp:Page",
  "properties": {...}
}
```

### Get Node by Path

Get a node by its path.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/{*path}
```

**Examples:**
```bash
# Get a node
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post

# Get as YAML
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post.yaml
```

**Response:**
```json
{
  "id": "abc123",
  "name": "my-post",
  "path": "/blog/my-post",
  "node_type": "myapp:Article",
  "properties": {
    "title": "My First Post"
  }
}
```

### List Children

List all children of a folder.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/{*path}/
```

**Example:**
```bash
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/
```

**Response:**
```json
[
  {
    "id": "post1",
    "name": "first-post",
    "path": "/blog/first-post",
    "node_type": "myapp:Article"
  },
  {
    "id": "post2",
    "name": "second-post",
    "path": "/blog/second-post",
    "node_type": "myapp:Article"
  }
]
```

### Deep Listing

Get nested children with depth control.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/{*path}?level={depth}
GET /api/repository/{repo}/{branch}/head/{workspace}/{*path}?level={depth}&flatten=true
```

**Query Parameters:**
- `level` - Depth to traverse (1-10)
- `flatten` - If true, returns flat map; if false, returns nested structure

**Examples:**
```bash
# Nested structure
curl 'http://localhost:8080/api/repository/myapp/main/head/content/blog?level=3'

# Flat map
curl 'http://localhost:8080/api/repository/myapp/main/head/content/blog?level=3&flatten=true'
```

**Nested Response:**
```json
{
  "/blog": {
    "node": {...},
    "children": {
      "/blog/2024": {
        "node": {...},
        "children": {}
      }
    }
  }
}
```

**Flat Response:**
```json
{
  "/blog": {...},
  "/blog/2024": {...},
  "/blog/2024/first-post": {...}
}
```

### Create Child Node

Create a node as a child of the path.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*parent_path}
Content-Type: application/json

{
  "name": "child-node",
  "node_type": "myapp:Page",
  "properties": {...}
}
```

**Query Parameters:**
- `deep=true` - Auto-create missing parent folders

**Example:**
```bash
curl -X POST 'http://localhost:8080/api/repository/myapp/main/head/content/blog?deep=true' \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-post",
    "node_type": "myapp:Article",
    "properties": {
      "title": "My First Post"
    }
  }'
```

**Response:**
```json
{
  "id": "newid",
  "name": "my-post",
  "path": "/blog/my-post",
  "node_type": "myapp:Article",
  "properties": {
    "title": "My First Post"
  }
}
```

### Update Node

Update a node's properties.

```http
PUT /api/repository/{repo}/{branch}/head/{workspace}/{*path}
Content-Type: application/json

{
  "id": "abc123",
  "name": "my-post",
  "path": "/blog/my-post",
  "node_type": "myapp:Article",
  "properties": {
    "title": "Updated Title"
  }
}
```

**Example:**
```bash
curl -X PUT http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post \
  -H "Content-Type: application/json" \
  -d '{
    "id": "abc123",
    "name": "my-post",
    "node_type": "myapp:Article",
    "properties": {
      "title": "Updated Title"
    }
  }'
```

**Response:**
```json
{"status": "ok"}
```

### Delete Node

Delete a node by path.

```http
DELETE /api/repository/{repo}/{branch}/head/{workspace}/{*path}
```

**Example:**
```bash
curl -X DELETE http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post
```

**Response:**
```json
{"deleted": true}
```

## Commands

Execute operations on nodes using the `raisin:cmd` path marker.

**Command Syntax:**
```
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/{command}
```

Commands can also be executed via query parameter for compatibility:
```
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}?command={command}
```

### Rename Node

Rename a node.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/rename
Content-Type: application/json

{
  "newName": "new-name"
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/old-name/raisin:cmd/rename \
  -H "Content-Type: application/json" \
  -d '{"newName": "new-name"}'
```

**Alternative (query parameter):**
```bash
curl -X POST 'http://localhost:8080/api/repository/myapp/main/head/content/blog/old-name?command=rename' \
  -H "Content-Type: application/json" \
  -d '{"newName": "new-name"}'
```

### Move Node

Move a node to a new location.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/move
Content-Type: application/json

{
  "targetPath": "/new/location"
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:cmd/move \
  -H "Content-Type: application/json" \
  -d '{"targetPath": "/archive/old-posts"}'
```

### Copy Node

Copy a single node (without children).

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/copy
Content-Type: application/json

{
  "targetPath": "/destination/folder",
  "newName": "copied-node"
}
```

**Parameters:**
- `targetPath` - Destination parent folder
- `newName` - Optional new name for the copy

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:cmd/copy \
  -H "Content-Type: application/json" \
  -d '{
    "targetPath": "/archive",
    "newName": "archived-post"
  }'
```

### Copy Tree

Copy a node with all its descendants.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/copy_tree
Content-Type: application/json

{
  "targetPath": "/destination/folder",
  "newName": "copied-folder"
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/2024/raisin:cmd/copy_tree \
  -H "Content-Type: application/json" \
  -d '{
    "targetPath": "/archive",
    "newName": "2024-backup"
  }'
```

### Publish Node

Publish a single node.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/publish
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:cmd/publish
```

**Response:**
```json
{}
```

### Publish Tree

Publish a node and all its descendants.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/publish_tree
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/2024/raisin:cmd/publish_tree
```

### Unpublish Node

Unpublish a single node.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/unpublish
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:cmd/unpublish
```

### Unpublish Tree

Unpublish a node and all its descendants.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/unpublish_tree
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/2024/raisin:cmd/unpublish_tree
```

### Reorder Child

Change the order of a child within its parent.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/{*path}/raisin:cmd/reorder
Content-Type: application/json

{
  "targetPath": "/parent/sibling-node",
  "movePosition": "after"
}
```

**Parameters:**
- `targetPath` - Path to sibling node
- `movePosition` - Either `"before"` or `"after"`

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:cmd/reorder \
  -H "Content-Type: application/json" \
  -d '{
    "targetPath": "/blog/other-post",
    "movePosition": "before"
  }'
```

## Property Path Access

Access and update specific properties within a node.

### Get Property

Get a specific property value.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@property.path
```

**Example:**
```bash
# Get title property
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post@title

# Get nested property
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post@author.email
```

**Response:**
```json
"My Post Title"
```

### Update Property

Update a specific property without replacing the entire node.

```http
PUT /api/repository/{repo}/{branch}/head/{workspace}/*path@property.path
Content-Type: application/json

"new value"
```

**Example:**
```bash
curl -X PUT http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post@title \
  -H "Content-Type: application/json" \
  -d '"Updated Title"'
```

**Response:**
```json
{"status": "property updated"}
```

## Version History

Access historical versions of nodes (requires versioning enabled).

### List Versions

Get all versions of a node using the `raisin:version` path marker.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path/raisin:version
```

**Example:**
```bash
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:version
```

**Response:**
```json
[
  {
    "id": "v1",
    "node_id": "abc123",
    "version": 1,
    "created_at": "2024-01-01T00:00:00Z"
  },
  {
    "id": "v2",
    "node_id": "abc123",
    "version": 2,
    "created_at": "2024-01-02T00:00:00Z"
  }
]
```

### Get Specific Version

Get a specific version of a node by version number.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path/raisin:version/:number
```

**Example:**
```bash
curl http://localhost:8080/api/repository/myapp/main/head/content/blog/my-post/raisin:version/2
```

**Response:**
```json
{
  "id": "v2",
  "node_id": "abc123",
  "version": 2,
  "node_data": {...},
  "created_at": "2024-01-02T00:00:00Z"
}
```

## File Upload/Download

Upload and download binary files.

### Upload File

Upload a file to a node property. If the node doesn't exist, a new `raisin:Asset` node
is automatically created with the filename as the `title` property.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/*path
Content-Type: multipart/form-data

<file data>
```

**Query Parameters:**
- `inline=true` - Store file content as string (max 11MB, must be valid UTF-8)
- `override_existing=true` - Replace existing file
- `commit_message` - Custom commit message for transaction
- `commit_actor` - Actor name for audit trail

**Example (external storage - creates node if needed):**
```bash
# Uploads logo.png and auto-creates a raisin:Asset node at /assets/logo
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/assets/logo \
  -F "file=@logo.png"
```

**Example (inline storage):**
```bash
curl -X POST 'http://localhost:8080/api/repository/myapp/main/head/content/docs/readme?inline=true' \
  -F "file=@README.md"
```

**Response:**
```json
{"status": "file uploaded"}
```

The file is stored as a `Resource` property on the node:

```json
{
  "properties": {
    "file": {
      "uuid": "resource-id",
      "name": "logo.png",
      "size": 12345,
      "mime_type": "image/png",
      "url": "/files/abc123.png",
      "metadata": {
        "storage_key": "abc123"
      },
      "is_loaded": true,
      "is_external": false
    }
  }
}
```

### Download File

Download a file from a node property as an attachment.

**Using command pattern (preferred):**
```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path/raisin:cmd/download
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@property/raisin:cmd/download
```

**Using query parameter:**
```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path?command=download
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@property?command=download
```

**Example:**
```bash
# Download from default "file" property using command pattern
curl http://localhost:8080/api/repository/myapp/main/head/content/assets/logo/raisin:cmd/download

# Download using query parameter
curl http://localhost:8080/api/repository/myapp/main/head/content/assets/logo?command=download

# Download from specific property
curl http://localhost:8080/api/repository/myapp/main/head/content/assets/banner@image/raisin:cmd/download
```

**Response:**
Binary file data with appropriate `Content-Type` and `Content-Disposition: attachment` headers.

### Stream File Inline (Auto-Detect)

Access a Resource property directly to stream content inline (without download prompt).
This is useful for displaying images, serving files to browsers, or embedding content.

```http
GET /api/repository/{repo}/{branch}/head/{workspace}/*path@property
```

When you access a property containing a `Resource` type, the content is automatically
streamed with the correct `Content-Type` header but **without** `Content-Disposition: attachment`.

**Behavior by property type:**

| Property Type | Response |
|---------------|----------|
| `Resource` (internal storage) | Streams binary content with `Content-Type` |
| `Resource` (external URL) | 307 redirect to the external URL |
| `String` (file-like property) | Returns as text with guessed `Content-Type` |
| Other types | Returns JSON |

**Example - Display image in browser:**
```bash
# Returns image with Content-Type: image/png (no download prompt)
curl http://localhost:8080/api/repository/myapp/main/head/content/assets/logo@file
```

**Example - Difference between streaming and download:**
```bash
# Stream inline (for embedding in pages, displaying images)
curl http://localhost:8080/api/repository/myapp/main/head/content/assets/logo@file
# Response: Content-Type: image/png

# Download as file (triggers browser download dialog)
curl http://localhost:8080/api/repository/myapp/main/head/content/assets/logo/raisin:cmd/download
# Response: Content-Type: image/png
#           Content-Disposition: attachment; filename="logo.png"
```

## NodeType Management

Manage NodeType schemas. NodeTypes are scoped by repository and branch.

### List All NodeTypes

Get all NodeTypes in a repository branch.

```http
GET /api/management/{repo}/{branch}/nodetypes
```

**Example:**
```bash
curl http://localhost:8080/api/management/myapp/main/nodetypes
```

**Response:**
```json
[
  {
    "name": "Article",
    "description": "Blog article",
    "properties": [...],
    "publishable": true
  }
]
```

### List Published NodeTypes

Get only published NodeTypes.

```http
GET /api/management/{repo}/{branch}/nodetypes/published
```

**Example:**
```bash
curl http://localhost:8080/api/management/myapp/main/nodetypes/published
```

### Create NodeType

Create a new NodeType.

```http
POST /api/management/{repo}/{branch}/nodetypes
Content-Type: application/json

{
  "name": "Article",
  "description": "A blog article",
  "properties": [
    {
      "name": "title",
      "property_type": "String",
      "required": true
    }
  ],
  "versionable": true,
  "publishable": true
}
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/management/myapp/main/nodetypes \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Article",
    "description": "Blog article",
    "properties": [
      {
        "name": "title",
        "property_type": "String",
        "required": true
      },
      {
        "name": "content",
        "property_type": "String",
        "required": true
      }
    ],
    "versionable": true,
    "publishable": true
  }'
```

### Get NodeType

Get a specific NodeType.

```http
GET /api/management/{repo}/{branch}/nodetypes/{name}
```

**Example:**
```bash
curl http://localhost:8080/api/management/myapp/main/nodetypes/Article
```

**Response:**
```json
{
  "name": "Article",
  "description": "Blog article",
  "properties": [...],
  "versionable": true,
  "publishable": true,
  "created_at": "2024-01-01T00:00:00Z"
}
```

### Get Resolved NodeType

Get a NodeType with inheritance resolved (includes properties from `extends` and `mixins`).

```http
GET /api/management/{repo}/{branch}/nodetypes/{name}/resolved
```

**Example:**
```bash
curl http://localhost:8080/api/management/myapp/main/nodetypes/Article/resolved
```

### Update NodeType

Update an existing NodeType.

```http
PUT /api/management/{repo}/{branch}/nodetypes/{name}
Content-Type: application/json

{
  "name": "Article",
  "description": "Updated description",
  "properties": [...]
}
```

**Example:**
```bash
curl -X PUT http://localhost:8080/api/management/myapp/main/nodetypes/Article \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Article",
    "description": "Updated blog article type",
    "properties": [...]
  }'
```

### Delete NodeType

Delete a NodeType.

```http
DELETE /api/management/{repo}/{branch}/nodetypes/{name}
```

**Example:**
```bash
curl -X DELETE http://localhost:8080/api/management/myapp/main/nodetypes/Article
```

### Publish NodeType

Publish a NodeType to make it available for use.

```http
POST /api/management/{repo}/{branch}/nodetypes/{name}/publish
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/management/myapp/main/nodetypes/Article/publish
```

### Unpublish NodeType

Unpublish a NodeType.

```http
POST /api/management/{repo}/{branch}/nodetypes/{name}/unpublish
```

**Example:**
```bash
curl -X POST http://localhost:8080/api/management/myapp/main/nodetypes/Article/unpublish
```

### Validate Node

Validate a node against its NodeType schema without saving it.

```http
POST /api/management/{repo}/{branch}/nodetypes/validate
Content-Type: application/json

{
  "name": "test-node",
  "node_type": "Article",
  "properties": {
    "title": "Test Article"
  }
}
```

**Note**: Pass a complete Node object (including `name` and `node_type` fields).

**Example:**
```bash
curl -X POST http://localhost:8080/api/management/myapp/main/nodetypes/validate \
  -H "Content-Type: application/json" \
  -d '{
    "name": "test-article",
    "node_type": "Article",
    "properties": {
      "title": "Test",
      "content": "Test content"
    }
  }'
```

**Response (valid):**
```json
{
  "valid": true,
  "message": "Node is valid"
}
```

**Response (invalid):**
```json
{
  "valid": false,
  "message": "Missing required property 'content' for NodeType 'Article'"
}
```

## Query Endpoints

Search and filter nodes.

### Simple Query

Query nodes by type, parent, or path with pagination.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/query
Content-Type: application/json

{
  "nodeType": "myapp:Article",
  "parent": "/blog",
  "limit": 10,
  "offset": 0
}
```

**Parameters:**
- `nodeType` - Filter by node type
- `parent` - Filter by parent path
- `path` - Get specific node by path (takes precedence)
- `limit` - Maximum results (default: all)
- `offset` - Offset for pagination (default: 0)

**Note**: You must provide at least one of: `path`, `parent`, or `nodeType`.

**Example:**
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query \
  -H "Content-Type: application/json" \
  -d '{
    "nodeType": "myapp:Article",
    "limit": 10
  }'
```

**Response:**
```json
{
  "items": [
    {
      "id": "abc123",
      "name": "my-article",
      "path": "/blog/my-article",
      "node_type": "myapp:Article",
      "properties": {...}
    }
  ],
  "page": {
    "total": 25,
    "limit": 10,
    "offset": 0,
    "nextOffset": 10
  }
}
```

### Advanced Query (DSL)

Query nodes using structured filter expressions with operators.

```http
POST /api/repository/{repo}/{branch}/head/{workspace}/query/dsl
Content-Type: application/json

{
  "and": [
    {
      "nodeType": {
        "eq": "myapp:Article"
      }
    },
    {
      "properties.published": {
        "eq": true
      }
    }
  ],
  "orderBy": {
    "path": "asc"
  },
  "limit": 10,
  "offset": 0
}
```

**Query Structure:**
- `and` - Array of filters (all must match)
- `or` - Array of filters (any must match)
- `not` - Negates a filter
- `orderBy` - Sort results by field(s)
- `limit` - Maximum results
- `offset` - Offset for pagination

**Field Operators:**
- `eq` - Equals
- `ne` - Not equals
- `like` - String contains (substring match)
- `in` - Value in array
- `gt` - Greater than
- `lt` - Less than
- `gte` - Greater than or equal
- `lte` - Less than or equal
- `exists` - Field exists (true) or doesn't exist (false)
- `contains` - Array contains value

**Examples:**

Filter by node type:
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query/dsl \
  -H "Content-Type: application/json" \
  -d '{
    "and": [
      {
        "nodeType": {
          "eq": "myapp:Article"
        }
      }
    ],
    "limit": 10
  }'
```

Filter by property value:
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query/dsl \
  -H "Content-Type: application/json" \
  -d '{
    "and": [
      {
        "properties.published": {
          "eq": true
        }
      },
      {
        "properties.author": {
          "eq": "John Doe"
        }
      }
    ]
  }'
```

Filter by path pattern:
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query/dsl \
  -H "Content-Type: application/json" \
  -d '{
    "and": [
      {
        "path": {
          "like": "/blog/"
        }
      }
    ]
  }'
```

Multiple conditions with OR:
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query/dsl \
  -H "Content-Type: application/json" \
  -d '{
    "or": [
      {
        "nodeType": {
          "eq": "myapp:Article"
        }
      },
      {
        "nodeType": {
          "eq": "myapp:Page"
        }
      }
    ]
  }'
```

NOT filter:
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query/dsl \
  -H "Content-Type: application/json" \
  -d '{
    "not": {
      "properties.published": {
        "eq": true
      }
    }
  }'
```

Range query:
```bash
curl -X POST http://localhost:8080/api/repository/myapp/main/head/content/query/dsl \
  -H "Content-Type: application/json" \
  -d '{
    "and": [
      {
        "properties.price": {
          "gte": 10
        }
      },
      {
        "properties.price": {
          "lte": 100
        }
      }
    ]
  }'
```

**Response:**
```json
{
  "items": [
    {
      "id": "abc123",
      "name": "my-article",
      "path": "/blog/my-article",
      "node_type": "myapp:Article",
      "properties": {
        "published": true,
        "author": "John Doe"
      }
    }
  ],
  "page": {
    "total": 5,
    "limit": 10,
    "offset": 0,
    "nextOffset": null
  }
}
```

## Workspace Management

Manage workspace configurations. Workspaces are scoped to a repository.

### List Workspaces

Get all workspaces for a repository.

```http
GET /api/workspaces/{repo}
```

**Example:**
```bash
curl http://localhost:8080/api/workspaces/myapp
```

**Response:**
```json
[
  {
    "name": "content",
    "allowed_node_types": ["raisin:Folder", "myapp:Page"],
    "allowed_root_node_types": ["raisin:Folder"]
  }
]
```

### Get Workspace

Get a specific workspace configuration.

```http
GET /api/workspaces/{repo}/{name}
```

**Example:**
```bash
curl http://localhost:8080/api/workspaces/myapp/content
```

**Response:**
```json
{
  "name": "content",
  "description": "Website content",
  "allowed_node_types": [
    "raisin:Folder",
    "myapp:Page",
    "myapp:Article"
  ],
  "allowed_root_node_types": [
    "raisin:Folder"
  ],
  "created_at": "2024-01-01T00:00:00Z"
}
```

### Create/Update Workspace

Create or update a workspace.

```http
PUT /api/workspaces/{repo}/{name}
Content-Type: application/json

{
  "name": "content",
  "description": "Website content",
  "allowed_node_types": [
    "raisin:Folder",
    "myapp:Page"
  ],
  "allowed_root_node_types": [
    "raisin:Folder"
  ]
}
```

**Example:**
```bash
curl -X PUT http://localhost:8080/api/workspaces/myapp/content \
  -H "Content-Type: application/json" \
  -d '{
    "name": "content",
    "description": "Website content",
    "allowed_node_types": [
      "raisin:Folder",
      "myapp:Page"
    ],
    "allowed_root_node_types": [
      "raisin:Folder"
    ]
  }'
```

## Audit Logs

Access audit logs for change tracking (requires audit adapter configured).

### Get Audit Logs by Node ID

Get all audit logs for a specific node.

```http
GET /api/audit/{repo}/{branch}/{workspace}/by-id/{id}
```

**Example:**
```bash
curl http://localhost:8080/api/audit/myapp/main/content/by-id/abc123
```

**Response:**
```json
[
  {
    "id": "log1",
    "node_id": "abc123",
    "action": "Update",
    "timestamp": "2024-01-01T00:00:00Z",
    "user_id": "user123"
  }
]
```

### Get Audit Logs by Path

Get audit logs for a node by its path.

```http
GET /api/audit/{repo}/{branch}/{workspace}/{*path}
```

**Example:**
```bash
curl http://localhost:8080/api/audit/myapp/main/content/blog/my-post
```

## Error Responses

The API uses standard HTTP status codes:

| Status | Description | Example |
|--------|-------------|---------|
| `200` | Success | Node retrieved successfully |
| `400` | Bad Request | Validation error, malformed JSON |
| `404` | Not Found | Node or NodeType not found |
| `500` | Internal Server Error | Database error, unexpected failure |

**Error Response Format:**

Most errors return a status code. Validation errors may include details in the response body:

```json
{
  "error": "Validation error: Missing required property 'title'"
}
```

**Common Validation Errors:**

- "Missing required property 'X'" - Required property not provided
- "NodeType 'X' not found" - Invalid node_type specified
- "Undefined property 'X' in strict mode" - Extra property in strict NodeType
- "Property 'X' must be unique" - Unique constraint violation
- "Parent node 'X' not found" - Invalid parent path
- "Cannot change node_type" - Attempted to change NodeType on update

## Response Formats

### JSON (Default)

```http
GET /api/repository/myapp/main/head/content/my-page
```

```json
{
  "id": "abc123",
  "name": "my-page",
  "path": "/my-page",
  "node_type": "myapp:Page",
  "properties": {...}
}
```

### YAML

Add `.yaml` or `.yml` extension to get YAML response:

```http
GET /api/repository/myapp/main/head/content/my-page.yaml
```

```yaml
id: abc123
name: my-page
path: /my-page
node_type: myapp:Page
properties:
  title: My Page
```

## Revision Routes (Read-Only)

Access historical snapshots of the repository at a specific revision. These routes are read-only.

### Get Root at Revision

```http
GET /api/repository/{repo}/{branch}/rev/{revision}/{workspace}/
```

### Get Node by ID at Revision

```http
GET /api/repository/{repo}/{branch}/rev/{revision}/{workspace}/$ref/{id}
```

### Get Node by Path at Revision

```http
GET /api/repository/{repo}/{branch}/rev/{revision}/{workspace}/{*path}
```

**Example:**
```bash
# Get the state of a node at a specific revision
curl http://localhost:8080/api/repository/myapp/main/rev/1234567890/content/blog/my-post
```

## Next Steps

- [Standalone Server Guide](../getting-started/server.md) - Running and configuring the server
- [Embedding Guide](../guides/embedding-guide.md) - Using NodeService directly in your Rust app
- [Multi-Tenant SaaS](../guides/multi-tenant-saas.md) - Building a multi-tenant system
