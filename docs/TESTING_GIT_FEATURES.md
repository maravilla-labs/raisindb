# Testing Git-Like Features in RaisinDB

This guide shows how to test all the Git-like features (repositories, branches, tags, versioning) using the integration tests.

## Running Integration Tests

### Option 1: Using raisin-server Integration Tests

These tests start an actual server and test end-to-end workflows using reqwest HTTP client.

```bash
# 1. Build the server
cargo build --package raisin-server --release

# 2. Run integration tests (with in-memory storage)
cargo test --package raisin-server --test integration_node_operations -- --ignored

# 3. Run integration tests (with RocksDB storage)
cargo test --package raisin-server --test integration_node_operations --features store-rocks -- --ignored
```

**What these tests cover:**
- Repository creation via HTTP API
- Branch creation and management
- Node operations (create, rename, move, copy, reorder)
- Versioning (manual versions, publishing)
- Tree operations (copy entire trees)

### Option 2: Using raisin-transport-http Unit Tests

These tests use tower's testing utilities (no actual server needed).

```bash
# Test branch and tag endpoints
cargo test -p raisin-transport-http --test http_branches_tags

# With RocksDB backend
cargo test -p raisin-transport-http --test http_branches_tags --features store-rocks
```

**What these tests cover:**
- Branch CRUD operations
- Tag CRUD operations
- HEAD pointer management
- Protected branch/tag handling

## Manual Testing Workflow

### 1. Start the Server

```bash
# In terminal 1
cargo run --package raisin-server --release

# Or with RocksDB backend
cargo run --package raisin-server --release --features store-rocks
```

### 2. Create a Repository

```bash
curl -X POST http://localhost:8080/api/repositories \
  -H "Content-Type: application/json" \
  -d '{
    "repo_id": "test-repo",
    "description": "Test repository for manual testing",
    "default_branch": "main"
  }'
```

**Expected response:** `201 Created`
```json
{
  "tenant_id": "default",
  "repo_id": "test-repo",
  "created_at": "2025-10-14T...",
  "branches": [],
  "config": {
    "default_branch": "main",
    "description": "Test repository for manual testing",
    "tags": {}
  }
}
```

### 3. Create Main Branch

```bash
curl -X POST http://localhost:8080/api/management/repositories/default/test-repo/branches \
  -H "Content-Type: application/json" \
  -d '{
    "branch_name": "main",
    "created_by": "admin",
    "from_revision": null,
    "protected": true
  }'
```

**Expected response:** `201 Created`

### 4. Create Content Nodes

```bash
# Create a folder
curl -X POST http://localhost:8080/api/repository/test-repo/main/demo/ \
  -H "Content-Type: application/json" \
  -d '{
    "name": "content",
    "node_type": "raisin:Folder"
  }'

# Create a page inside the folder
curl -X POST http://localhost:8080/api/repository/test-repo/main/demo/content \
  -H "Content-Type: application/json" \
  -d '{
    "name": "hello-world",
    "node_type": "raisin:Page",
    "properties": {
      "title": "Hello World",
      "content": "This is a test page"
    }
  }'
```

### 5. Test Versioning

```bash
# Create a manual version
curl -X POST http://localhost:8080/api/repository/test-repo/main/demo/content/hello-world/raisin%3Acmd/create_version \
  -H "Content-Type: application/json" \
  -d '{
    "note": "Initial version"
  }'

# Publish the node (creates automatic version)
curl -X POST http://localhost:8080/api/repository/test-repo/main/demo/content/hello-world/raisin%3Acmd/publish \
  -H "Content-Type: application/json" \
  -d '{}'

# List all versions
curl http://localhost:8080/api/repository/test-repo/main/demo/content/hello-world/raisin%3Aversion
```

**Expected response:**
```json
[
  {
    "version": 1,
    "created_at": "...",
    "note": "Initial version",
    "snapshot": { /* node data */ }
  },
  {
    "version": 2,
    "created_at": "...",
    "note": "Published",
    "snapshot": { /* node data */ }
  }
]
```

### 6. Create a Tag

```bash
curl -X POST http://localhost:8080/api/management/repositories/default/test-repo/tags \
  -H "Content-Type: application/json" \
  -d '{
    "tag_name": "v1.0.0",
    "revision": 1,
    "created_by": "admin",
    "message": "First stable release",
    "protected": true
  }'

# List all tags
curl http://localhost:8080/api/management/repositories/default/test-repo/tags
```

### 7. Test Copy Operations

```bash
# Copy a node
curl -X POST http://localhost:8080/api/repository/test-repo/main/demo/content/hello-world/raisin%3Acmd/copy \
  -H "Content-Type: application/json" \
  -d '{
    "target_path": "/content/hello-copy",
    "deep": false
  }'

# Verify the copy
curl http://localhost:8080/api/repository/test-repo/main/demo/content/hello-copy
```

### 8. Create Development Branch

```bash
curl -X POST http://localhost:8080/api/management/repositories/default/test-repo/branches \
  -H "Content-Type: application/json" \
  -d '{
    "branch_name": "develop",
    "created_by": "admin",
    "from_revision": null,
    "protected": false
  }'

# List all branches
curl http://localhost:8080/api/management/repositories/default/test-repo/branches
```

## Verifying Storage Backend

### Check In-Memory Storage

In-memory storage is the default. Data is lost when the server stops.

```bash
# Start server (default is in-memory)
cargo run --package raisin-server

# Check that data disappears after restart
curl http://localhost:8080/api/repositories
# Should show repositories

# Restart server (Ctrl+C and run again)
cargo run --package raisin-server

# Check again
curl http://localhost:8080/api/repositories
# Should be empty
```

### Check RocksDB Storage

RocksDB persists data to disk in `.data/raisindb` directory.

```bash
# Start server with RocksDB
cargo run --package raisin-server --features store-rocks

# Create some data
curl -X POST http://localhost:8080/api/repositories \
  -H "Content-Type: application/json" \
  -d '{"repo_id": "persistent-test"}'

# Restart server
# Ctrl+C then run again
cargo run --package raisin-server --features store-rocks

# Data should still exist
curl http://localhost:8080/api/repositories
# Should show "persistent-test"

# Check the database directory
ls -la .data/raisindb/
```

## Troubleshooting

### Port Already in Use

```bash
# Kill any existing server
pkill -9 raisin-server

# Or use a different port
RAISIN_SERVER_PORT=8081 cargo run --package raisin-server
```

### RocksDB Lock Error

```bash
# Remove the lock file
rm -rf .data/raisindb/LOCK

# Or use a fresh database
rm -rf .data/raisindb
cargo run --package raisin-server --features store-rocks
```

### Integration Tests Failing

```bash
# Make sure no server is running
pkill -9 raisin-server

# Clean build
cargo clean
cargo build --package raisin-server --release

# Run tests
cargo test --package raisin-server --test integration_node_operations -- --ignored
```

## What's Tested

### Repository Management ✅
- Create repository
- List repositories
- Get repository details
- Update repository config
- Delete repository

### Branch Management ✅
- Create branch
- List branches
- Get branch details
- Update HEAD pointer
- Delete branch
- Protected branches

### Tag Management ✅
- Create tag
- List tags
- Get tag details
- Delete tag
- Protected tags

### Node Operations ✅
- Create nodes
- Rename nodes (with published checks)
- Move nodes (with published checks)
- Copy nodes
- Copy trees (deep copy)
- Reorder children
- Delete nodes

### Versioning & Publishing ✅
- Create manual versions
- Auto-versioning on publish
- List versions
- Get specific version
- Update version notes
- Version snapshots capture draft state

## Performance Testing

For load testing:

```bash
# Install Apache Bench
sudo apt-get install apache2-utils

# Test repository creation
ab -n 1000 -c 10 -p repo.json -T application/json \
  http://localhost:8080/api/repositories

# Where repo.json contains:
# {"repo_id": "perf-test", "description": "Performance test"}
```

## See Also

- [API_REPOSITORIES.md](./API_REPOSITORIES.md) - Repository management API reference
- [API_BRANCHES_TAGS.md](./API_BRANCHES_TAGS.md) - Branch and tag API reference
- [API_QUICK_REFERENCE.md](./API_QUICK_REFERENCE.md) - Complete API reference
