# raisin-binary

Pluggable binary storage abstraction for RaisinDB.

## Overview

This crate provides a trait-based storage interface for binary data (files, uploads, attachments) with built-in security protections and multi-tenant support.

## Components

- `BinaryStorage` trait - Storage interface for put/get/delete operations
- `FilesystemBinaryStorage` - Local filesystem backend (default)
- `S3BinaryStorage` - S3-compatible backend (Cloudflare R2, AWS S3, MinIO)
- `StoredObject` - Metadata returned after storing (key, URL, size, MIME type)

## Features

```toml
[dependencies]
raisin-binary = { path = "../raisin-binary" }                    # fs only
raisin-binary = { path = "../raisin-binary", features = ["s3"] } # with S3
```

| Feature | Description |
|---------|-------------|
| `fs` (default) | Filesystem storage backend |
| `s3` | S3-compatible storage (requires `aws-sdk-s3`) |

## Usage

```rust
use raisin_binary::{BinaryStorage, FilesystemBinaryStorage};

let storage = FilesystemBinaryStorage::new("/var/data/uploads", Some("https://cdn.example.com".into()));

// Store bytes
let obj = storage.put_bytes(
    b"hello world",
    Some("text/plain"),
    Some("txt"),
    Some("greeting.txt"),
    Some("tenant-123"),  // multi-tenant isolation
).await?;

// Retrieve
let data = storage.get(&obj.key).await?;

// Get as file path (zero-copy for fs, temp file for S3)
let (path, is_temp) = storage.get_as_path(&obj.key).await?;
// ... use path ...
if is_temp { tokio::fs::remove_file(&path).await.ok(); }

// Delete
storage.delete(&obj.key).await?;
```

## Security

- **Path traversal protection**: All keys validated against `..`, absolute paths, and drive letters
- **Multi-tenant isolation**: Optional tenant prefix in storage keys

## Integration

Used by:
- `raisin-server` - File upload endpoints
- `raisin-ai` - PDF processing from storage
- `raisin-transport-ws` - WebSocket binary operations
- `raisin-rocksdb` - Binary storage callbacks

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
