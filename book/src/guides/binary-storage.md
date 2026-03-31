# Binary Storage

The `raisin-binary` crate provides a pluggable storage backend for binary data (files, images, documents) with support for local filesystem and Amazon S3.

## Storage Backends

### Local Filesystem

The default backend stores files on the local filesystem:

```toml
[dependencies]
raisin-binary = { path = "../raisin-binary", features = ["fs"] }
```

Files are stored with generated keys to prevent path traversal attacks. The storage layer validates all keys to reject:
- Empty keys
- Absolute paths
- Parent directory references (`..`)
- Windows drive letters

### Amazon S3

For production deployments, the S3 backend provides scalable object storage:

```toml
[dependencies]
raisin-binary = { path = "../raisin-binary", features = ["s3"] }
```

## BinaryStorage Trait

Both backends implement the `BinaryStorage` trait:

```rust
pub trait BinaryStorage: Send + Sync {
    /// Store a binary stream
    fn put_stream<'a, S>(
        &'a self,
        stream: S,
        content_type: Option<&'a str>,
        ext: Option<&'a str>,
        original_name: Option<&'a str>,
        size_hint: Option<u64>,
        tenant_context: Option<&'a str>,
    ) -> /* ... */ StoredObject;

    /// Retrieve a stored object
    fn get(&self, key: &str) -> /* ... */ Bytes;

    /// Delete a stored object
    fn delete(&self, key: &str) -> /* ... */;
}
```

## StoredObject

Every stored file returns metadata:

```rust
pub struct StoredObject {
    pub key: String,                    // Storage key
    pub url: String,                    // Accessible URL
    pub name: Option<String>,           // Original filename
    pub size: i64,                      // Size in bytes
    pub mime_type: Option<String>,      // MIME type
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

## Multi-Tenant Isolation

When `tenant_context` is provided, binary storage automatically isolates files per tenant, preventing cross-tenant access to uploaded content.

## Feature Flags

| Feature | Default | Description |
|---------|---------|-------------|
| `fs` | Yes | Local filesystem storage |
| `s3` | No | Amazon S3 storage |
