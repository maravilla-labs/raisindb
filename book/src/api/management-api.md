# Management API Reference

The Management API provides endpoints for system administration, monitoring, and maintenance operations. Management endpoints are split across several route groups:

- **Admin/Database management**: `/api/admin/management/database/...` (repository-specific indexes)
- **Admin/Global management**: `/api/admin/management/global/...` (RocksDB operations)
- **Admin/Tenant management**: `/api/admin/management/tenant/...` (tenant-level cleanup)
- **System updates**: `/api/management/repositories/...` (check/apply updates)

> **Note**: The admin Management API endpoints require the RocksDB storage backend (`storage-rocksdb` feature).

## Base URL

```
http://localhost:8080
```

## Database Management Endpoints

Repository-specific index management operations. These endpoints are scoped to a specific tenant and repository.

### Full-Text Index Health

Check the health of the full-text search index.

```http
GET /api/admin/management/database/{tenant}/{repo}/fulltext/health
```

### Verify Full-Text Index

Verify the integrity of the full-text search index.

```http
POST /api/admin/management/database/{tenant}/{repo}/fulltext/verify
```

### Rebuild Full-Text Index

Rebuild the full-text search index from scratch.

```http
POST /api/admin/management/database/{tenant}/{repo}/fulltext/rebuild
```

### Optimize Full-Text Index

Optimize the full-text search index for better query performance.

```http
POST /api/admin/management/database/{tenant}/{repo}/fulltext/optimize
```

### Purge Full-Text Index

Purge all data from the full-text search index.

```http
POST /api/admin/management/database/{tenant}/{repo}/fulltext/purge
```

### Vector Index Health

Check the health of the vector search index.

```http
GET /api/admin/management/database/{tenant}/{repo}/vector/health
```

### Verify Vector Index

Verify the integrity of the vector search index.

```http
POST /api/admin/management/database/{tenant}/{repo}/vector/verify
```

### Rebuild Vector Index

Rebuild the vector search index from scratch.

```http
POST /api/admin/management/database/{tenant}/{repo}/vector/rebuild
```

### Regenerate Vector Embeddings

Regenerate all vector embeddings for the repository.

```http
POST /api/admin/management/database/{tenant}/{repo}/vector/regenerate
```

### Optimize Vector Index

Optimize the vector search index.

```http
POST /api/admin/management/database/{tenant}/{repo}/vector/optimize
```

### Restore Vector Index

Restore the vector search index from stored data.

```http
POST /api/admin/management/database/{tenant}/{repo}/vector/restore
```

### Start Reindex

Start a full reindex operation for all RocksDB indexes.

```http
POST /api/admin/management/database/{tenant}/{repo}/reindex/start
```

### Verify Relation Integrity

Check the integrity of relation indexes.

```http
POST /api/admin/management/database/{tenant}/{repo}/relations/verify
```

### Repair Relation Integrity

Repair any inconsistencies found in relation indexes.

```http
POST /api/admin/management/database/{tenant}/{repo}/relations/repair
```

## Global Management Endpoints

System-wide RocksDB operations.

### Compact RocksDB

Trigger manual compaction of the RocksDB database.

```http
POST /api/admin/management/global/rocksdb/compact
```

### Backup RocksDB

Create a backup of the entire RocksDB database.

```http
POST /api/admin/management/global/rocksdb/backup
```

### Get RocksDB Stats

Retrieve RocksDB performance statistics.

```http
GET /api/admin/management/global/rocksdb/stats
```

## Tenant Management Endpoints

### Cleanup Tenant

Clean up orphaned data and perform maintenance for a tenant.

```http
POST /api/admin/management/tenant/{tenant}/cleanup
```

### Get Tenant Stats

Get storage statistics for a specific tenant.

```http
GET /api/admin/management/tenant/{tenant}/stats
```

## System Updates Endpoints

Check for and apply built-in system updates for a repository.

### Get Pending Updates

List pending system updates for a repository.

```http
GET /api/management/repositories/{tenant_id}/{repo_id}/system-updates
```

### Apply Updates

Apply pending system updates for a repository.

```http
POST /api/management/repositories/{tenant_id}/{repo_id}/system-updates/apply
```

## Error Responses

When an error occurs, the API returns an appropriate HTTP status code and error message.

## curl Examples

### Get RocksDB Stats
```bash
curl http://localhost:8080/api/admin/management/global/rocksdb/stats
```

### Compact RocksDB
```bash
curl -X POST http://localhost:8080/api/admin/management/global/rocksdb/compact
```

### Backup RocksDB
```bash
curl -X POST http://localhost:8080/api/admin/management/global/rocksdb/backup
```

### Check Full-Text Index Health
```bash
curl http://localhost:8080/api/admin/management/database/my-tenant/my-repo/fulltext/health
```

### Rebuild Full-Text Index
```bash
curl -X POST http://localhost:8080/api/admin/management/database/my-tenant/my-repo/fulltext/rebuild
```

### Start Reindex
```bash
curl -X POST http://localhost:8080/api/admin/management/database/my-tenant/my-repo/reindex/start
```

### Cleanup Tenant
```bash
curl -X POST http://localhost:8080/api/admin/management/tenant/my-tenant/cleanup
```

### Get Tenant Stats
```bash
curl http://localhost:8080/api/admin/management/tenant/my-tenant/stats
```

### Check Pending System Updates
```bash
curl http://localhost:8080/api/management/repositories/my-tenant/my-repo/system-updates
```

### Apply System Updates
```bash
curl -X POST http://localhost:8080/api/management/repositories/my-tenant/my-repo/system-updates/apply
```

## Security Considerations

1. **Access Control**: Management endpoints should be protected with authentication in production environments.

2. **Network Security**: Consider exposing management endpoints only on internal networks or through VPN.

3. **Resource Limits**: Long-running operations like index rebuilds can be resource-intensive. Monitor system resources during these operations.

## See Also

- [Storage Backends](../architecture/storage-backends.md)
- [REST API](./rest-api.md)