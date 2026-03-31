# raisin-embeddings

Vector embeddings support for RaisinDB.

## Overview

This crate provides infrastructure for managing vector embeddings, including tenant-level configuration, secure API key storage, embedding providers, and storage abstractions.

## Features

- **Tenant Configuration** - Per-tenant embedding settings (provider, model, dimensions)
- **Secure API Key Storage** - AES-256-GCM encryption for API keys
- **Multiple Providers** - OpenAI, Claude (Voyage), Ollama support
- **Embedding Storage** - Traits for storing/retrieving embedding vectors
- **Job Queue** - Background job system for embedding generation
- **Chunking Support** - Split large text into smaller chunks via `raisin-ai`

## Usage

### Tenant Configuration

```rust
use raisin_embeddings::{TenantEmbeddingConfig, EmbeddingProvider};

let mut config = TenantEmbeddingConfig::new("my-tenant".to_string());
config.enabled = true;
config.provider = EmbeddingProvider::OpenAI;
config.model = "text-embedding-3-small".to_string();
config.dimensions = 1536;
```

### API Key Encryption

```rust
use raisin_embeddings::ApiKeyEncryptor;

let master_key = [0u8; 32]; // Use secure key in production
let encryptor = ApiKeyEncryptor::new(&master_key);

// Encrypt
let encrypted = encryptor.encrypt("sk-my-api-key")?;

// Decrypt
let decrypted = encryptor.decrypt(&encrypted)?;
```

### Generating Embeddings

```rust
use raisin_embeddings::{create_provider, EmbeddingProvider};

let provider = create_provider(
    &EmbeddingProvider::OpenAI,
    "sk-api-key",
    "text-embedding-3-small"
)?;

// Single embedding
let vector = provider.generate_embedding("Hello world").await?;

// Batch embeddings
let vectors = provider.generate_embeddings_batch(&texts).await?;
```

### Embedding Jobs

```rust
use raisin_embeddings::{EmbeddingJob, EmbeddingJobKind};

// Create job for new node
let job = EmbeddingJob::add_node(
    tenant_id, repo_id, branch, workspace_id, node_id, revision
);

// Create job for deleted node
let job = EmbeddingJob::delete_node(
    tenant_id, repo_id, branch, workspace_id, node_id, revision
);

// Create job for branch copy
let job = EmbeddingJob::branch_created(
    tenant_id, repo_id, new_branch, workspace_id, source_branch, revision
);
```

## Components

| Module | Description |
|--------|-------------|
| `config.rs` | `TenantEmbeddingConfig`, `EmbeddingProvider` enum |
| `crypto.rs` | `ApiKeyEncryptor` with AES-256-GCM |
| `provider.rs` | `EmbeddingProvider` trait, `OpenAIProvider` |
| `models.rs` | `EmbeddingData`, `EmbeddingJob`, `EmbeddingJobKind` |
| `storage.rs` | `TenantEmbeddingConfigStore` trait |
| `embedding_storage.rs` | `EmbeddingStorage`, `EmbeddingJobStore` traits |

## Supported Providers

| Provider | Models | Dimensions |
|----------|--------|------------|
| OpenAI | `text-embedding-ada-002` | 1536 |
| OpenAI | `text-embedding-3-small` | 1536 |
| OpenAI | `text-embedding-3-large` | 3072 |
| Claude | Voyage (planned) | - |
| Ollama | Local models (planned) | - |

## Storage Key Format

Embeddings are stored in RocksDB with the following key format:

```
{tenant}\0{repo}\0{branch}\0{workspace}\0{embedder_hash:11}\0{kind:1}\0{source_id}\0{chunk_idx:04}\0{revision:HLC:16bytes}
```

- `embedder_hash` - Stable 11-char hash identifying the embedding model
- `kind` - Embedding type (text/image)
- `chunk_idx` - Chunk index for multi-chunk embeddings
- `revision` - Full HLC in descending order (latest first)

## Unified AI Provider Integration

The preferred configuration uses references to `TenantAIConfig`:

```rust
let mut config = TenantEmbeddingConfig::new("tenant".to_string());
config.ai_provider_ref = Some("openai".to_string());
config.ai_model_ref = Some("text-embedding-3-small".to_string());
// API key comes from TenantAIConfig, not stored here
```

## Security

- API keys encrypted with AES-256-GCM before storage
- Master keys should be stored securely (env vars, secrets manager)
- Encrypted keys never returned to clients
- Separate storage recommended for enhanced security

## Crate Usage

Used by:
- `raisin-server` - Embedding event handler and worker
- `raisin-rocksdb` - Storage implementation, job handlers
- `raisin-transport-http` - Embedding API endpoints, hybrid search
- `raisin-sql-execution` - Vector search in SQL queries

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
