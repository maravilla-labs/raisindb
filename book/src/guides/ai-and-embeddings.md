# AI, Embeddings, and Vector Search

RaisinDB integrates AI/LLM capabilities, vector embeddings, and approximate nearest neighbor search through three crates: `raisin-ai` (provider management), `raisin-embeddings` (embedding storage and generation), and `raisin-hnsw` (HNSW vector index).

## AI Provider Management

The `raisin-ai` crate provides tenant-level AI configuration with support for multiple LLM providers.

### Supported Providers

| Provider | Models | Notes |
|----------|--------|-------|
| OpenAI | GPT-4, GPT-3.5, etc. | Default provider |
| Anthropic | Claude models | |
| Google Gemini | Gemini 1.5, 2.0 | Tool calling support |
| Azure OpenAI | Azure-hosted OpenAI | Enterprise deployments |
| Groq | Open-source models | Fast inference |
| OpenRouter | Multi-provider router | Unified API |
| AWS Bedrock | Claude, Nova, Llama | Fully implemented, default provider |
| Ollama | Local models | Self-hosted |
| Custom | Any OpenAI-compatible | Custom endpoint |

### Configuration

Each tenant configures AI independently:

```rust
use raisin_ai::config::{TenantAIConfig, AIProviderConfig, AIProvider};
use raisin_ai::crypto::ApiKeyEncryptor;

// Encrypt API key with AES-256-GCM
let master_key = [0u8; 32]; // Use a secure key in production
let encryptor = ApiKeyEncryptor::new(&master_key);
let encrypted = encryptor.encrypt("sk-my-api-key").unwrap();

// Configure a provider
let provider_config = AIProviderConfig {
    provider: AIProvider::OpenAI,
    api_key_encrypted: Some(encrypted),
    api_endpoint: None,
    enabled: true,
    models: vec![],
};

let config = TenantAIConfig {
    tenant_id: "my-tenant".to_string(),
    providers: vec![provider_config],
};
```

### Use Cases

Models can be assigned to specific use cases:

| Use Case | Description |
|----------|-------------|
| `Chat` | Interactive chat conversations |
| `Completion` | Text completion and generation |
| `Embedding` | Vector embedding generation |
| `Agent` | Agentic workflows with tool calling |
| `Classification` | Content classification and labeling |

### API Key Security

API keys are encrypted using AES-256-GCM before storage. The master encryption key should be stored securely (environment variables, secrets manager). Encrypted keys are never returned to clients.

### SQL Configuration

All AI and embedding configuration can also be managed via SQL (see [SQL Reference](../api/sql-reference.md#ai--embedding-configuration)):

```sql
-- Configure embedding provider via SQL
ALTER EMBEDDING CONFIG
  SET PROVIDER = 'OpenAI'
  SET MODEL = 'text-embedding-3-small'
  SET API_KEY = 'sk-...'
  SET ENABLED = true;

-- View configuration
SHOW EMBEDDING CONFIG;

-- Test connection
TEST EMBEDDING CONNECTION;

-- View AI providers
SHOW AI PROVIDERS;
```

## Completions

The `AIProviderTrait` provides a unified interface for completions across all providers.

### Completion Request

```rust
use raisin_ai::{CompletionRequest, Message, Role};

let request = CompletionRequest {
    model: "gpt-4o".to_string(),
    messages: vec![
        Message::system("You are a helpful assistant."),
        Message::user("Summarize this document."),
    ],
    temperature: Some(0.7),
    max_tokens: Some(500),
    tools: None,
    response_format: None,
};

let response = provider.complete(request).await?;
println!("{}", response.content);
```

### Multimodal Messages

Messages can include both text and images:

```rust
use raisin_ai::{Message, ContentPart};

let message = Message::user_multimodal(vec![
    ContentPart::text("What's in this image?"),
    ContentPart::image_url("https://example.com/photo.jpg"),
    ContentPart::image_base64("data:image/png;base64,iVBOR..."),
]);
```

### Streaming

Streaming completions return chunks as they're generated:

```rust
use raisin_ai::{accumulate_stream, StreamEvent};

let stream = provider.stream_complete(request).await?;

// Accumulate with event callbacks
let response = accumulate_stream(stream, |event| {
    match event {
        StreamEvent::TextChunk(text) => print!("{}", text),
        StreamEvent::ThoughtChunk(thought) => {}, // model reasoning
    }
}).await?;
```

### Tool Calling / Function Calling

Define tools that AI models can call during completions:

```rust
use raisin_ai::{ToolDefinition, ToolCall};

let tools = vec![
    ToolDefinition::function(
        "search_products",
        "Search the product catalog",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "category": { "type": "string" }
            },
            "required": ["query"]
        }),
    ),
];

let request = CompletionRequest {
    model: "gpt-4o".to_string(),
    messages: vec![Message::user("Find me running shoes under $100")],
    tools: Some(tools),
    ..Default::default()
};

let response = provider.complete(request).await?;

// Check for tool calls in the response
if let Some(tool_calls) = response.tool_calls {
    for call in tool_calls {
        println!("Call: {} with args: {}", call.function.name, call.function.arguments);
    }
}
```

After executing tool calls, feed results back:

```rust
messages.push(Message::assistant_with_tool_calls(tool_calls));
messages.push(Message::tool("search_products", tool_call_id, results_json));
// Continue the conversation with updated messages
```

For models that emit tool calls in raw text (e.g., `<function=name>{args}</function>`), the `extract_tool_calls_from_content()` utility parses them. The `StreamingToolCallDetector` handles incremental detection during streaming.

Auto-generated system prompts for tool usage are available via `ToolDefinition::generate_tool_guidance()`.

### Structured Output (JSON Schema Mode)

Request structured JSON output conforming to a schema:

```rust
use raisin_ai::ResponseFormat;

let request = CompletionRequest {
    response_format: Some(ResponseFormat::JsonSchema {
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "sentiment": { "type": "string", "enum": ["positive", "negative", "neutral"] },
                "confidence": { "type": "number" }
            }
        }),
    }),
    ..Default::default()
};
```

Output validation is available via `validate_output()`.

### Provider Capabilities

Query provider capabilities at runtime:

```rust
// Check what a provider supports
provider.supports_streaming();
provider.supports_tools();
provider.list_available_models().await?;
```

## Text Chunking

The `TextChunker` splits documents into chunks suitable for embedding generation:

```rust
use raisin_ai::{TextChunker, ChunkingConfig, SplitterType, OverlapConfig};

let config = ChunkingConfig {
    splitter: SplitterType::Recursive,  // or FixedSize
    chunk_size: 512,                     // tokens
    overlap: OverlapConfig { tokens: 50 },
};

let chunker = TextChunker::new(config);
let chunks: Vec<TextChunk> = chunker.chunk("Long document text...");
```

Chunking is token-aware (uses tiktoken) and supports configurable overlap between chunks.

### Automatic Document Chunking in the Embedding Pipeline

When chunking configuration is set on the embedding config, the embedding pipeline automatically chunks long documents before generating embeddings. Each chunk is stored as a separate vector in the HNSW index, linked back to the source document. This enables accurate retrieval for documents that exceed the embedding model's context window.

Configure chunking via SQL:

```sql
ALTER EMBEDDING CONFIG
  SET CHUNKING_ENABLED = true
  SET CHUNK_SIZE = 512
  SET CHUNK_OVERLAP = 50;
```

Use `SearchMode::Chunks` in vector search to return individual chunks, or `SearchMode::Documents` (default) to deduplicate results by source document.

## PDF Processing

With the `pdf` or `pdf-markdown` feature flags, documents can be extracted from PDF files:

```toml
[dependencies]
raisin-ai = { path = "../raisin-ai", features = ["pdf"] }
```

Multiple extraction strategies:
- **Native** (`pdf-extract`) -- fast text extraction from text-based PDFs
- **Markdown** (`pdf_oxide`) -- preserves structure as Markdown
- **OCR** (`tesseract`) -- for scanned PDFs, configurable via `OcrOptions`

The `PdfProcessor` uses a router pattern to select the best strategy per document.

## Local Inference (Candle)

The optional `candle` feature enables local AI inference using the Candle framework:

```toml
[dependencies]
raisin-ai = { path = "../raisin-ai", features = ["candle"] }
```

**Supported local models:**
- **CLIP** -- image and text embeddings for multimodal search
- **BLIP** -- image captioning
- **Moondream** -- image understanding

Models are downloaded from HuggingFace on first use and cached locally. No external API calls required.

## Embeddings

The `raisin-embeddings` crate manages vector embedding storage and generation.

### Embedding Providers

RaisinDB supports multiple embedding providers out of the box:

| Provider | Models | Dimensions | Notes |
|----------|--------|------------|-------|
| **OpenAI** | text-embedding-3-small, text-embedding-3-large, text-embedding-ada-002 | 1536, 3072, 1536 | Default, most popular |
| **Voyage AI** (Claude) | voyage-large-2-instruct, voyage-code-2, voyage-3, voyage-3-lite | 1024, 1536, 1024, 512 | Optimized for code and semantic search |
| **Ollama** | nomic-embed-text, all-minilm, mxbai-embed-large, snowflake-arctic-embed | 768, 384, 1024, 1024 | Local or remote, no API key required for local |
| **HuggingFace** | Local candle models | Varies | Coming soon (CLIP image embeddings available) |

### Configuration via Admin Console

The easiest way to configure embeddings is through the admin console:

1. Navigate to **Tenant Settings > Embeddings**
2. Select a provider (OpenAI, Voyage AI, or Ollama)
3. Enter your API key (optional for local Ollama)
4. For Ollama: optionally set a **Base URL** for remote instances (default: `http://localhost:11434`)
5. Select a model -- dimensions are set automatically
6. Click **Test Connection** to verify the provider is reachable
7. Enable embeddings and save

### Configuration via API

```bash
# Set embedding config
curl -X POST /api/tenants/{tenant}/embeddings/config \
  -H 'Content-Type: application/json' \
  -d '{
    "enabled": true,
    "provider": "OpenAI",
    "model": "text-embedding-3-small",
    "dimensions": 1536,
    "api_key_plain": "sk-...",
    "include_name": true,
    "include_path": true,
    "max_embeddings_per_repo": null
  }'

# For Ollama (local, no API key needed)
curl -X POST /api/tenants/{tenant}/embeddings/config \
  -d '{
    "enabled": true,
    "provider": "Ollama",
    "model": "nomic-embed-text",
    "dimensions": 768,
    "include_name": true,
    "include_path": true
  }'

# For remote Ollama (with optional base URL and API key)
curl -X POST /api/tenants/{tenant}/embeddings/config \
  -d '{
    "enabled": true,
    "provider": "Ollama",
    "model": "nomic-embed-text",
    "dimensions": 768,
    "base_url": "https://ollama.mycompany.com",
    "api_key_plain": "optional-auth-token",
    "include_name": true,
    "include_path": true
  }'

# Test connection
curl -X POST /api/tenants/{tenant}/embeddings/config/test
```

### Schema-Driven Indexing

Control which node types and properties are embedded via NodeType schemas:

```yaml
name: myapp:Article
indexable: true
index_types: [Fulltext, Vector]
properties:
  - name: title
    type: String
    index: [Fulltext, Vector]   # Included in embeddings
  - name: body
    type: String
    index: [Fulltext, Vector]   # Included in embeddings
  - name: author
    type: String
    index: [Property]           # Property index only, not embedded
  - name: thumbnail
    type: Resource              # Resources are not embeddable
```

Properties with `index: [Vector]` are included in the text sent to the embedding provider. The `include_name` and `include_path` config options control whether the node's name and path are prepended to the embedding text.

### Provider Abstraction

The `EmbeddingProvider` trait abstracts over different embedding APIs:

```rust
use raisin_embeddings::{create_provider, config::EmbeddingProvider};

// Create OpenAI provider
let provider = create_provider(
    &EmbeddingProvider::OpenAI,
    "sk-your-api-key",
    "text-embedding-3-small",
)?;
let vector = provider.generate_embedding("Hello world").await?;

// Create Ollama provider (no API key)
let provider = create_provider(
    &EmbeddingProvider::Ollama,
    "",
    "nomic-embed-text",
)?;

// Test connection (calls provider with "test" input)
let dims = provider.test_connection().await?;
println!("Connected! Dimensions: {}", dims);
```

### Embedding Storage

Embeddings are stored per-node with multi-model support:

```rust
use raisin_embeddings::{EmbeddingStorage, EmbeddingData};

let embedding = EmbeddingData {
    node_id: "node-123".to_string(),
    vector: vec![0.1, 0.2, 0.3, /* ... */],
    model: "text-embedding-3-small".to_string(),
    // ...
};
```

### Embedding Versioning

The `EmbedderId` prevents collisions when the embedding model or configuration changes:

```rust
use raisin_ai::EmbedderId;

// EmbedderId encodes provider + model + dimensions + tokenizer
let id = EmbedderId::new("openai", "text-embedding-3-small", 1536, "cl100k_base");

// Generates a stable 11-char base64url hash for storage keys
let key = id.to_key_hash(); // e.g., "a1B2c3D4e5F"
```

When you switch embedding models, old and new embeddings coexist without conflicts.

### Embedding Jobs

Embedding generation runs asynchronously through the job system. When a node is created or updated, an `EmbeddingGenerate` job is enqueued automatically (if the node type is configured for vector indexing). The background worker processes these jobs, calling the configured embedding provider and storing the result in both RocksDB and the HNSW index.

The embedding worker retries transient failures (network errors, rate limits, provider timeouts) with exponential backoff. Failed jobs are retried up to a configurable maximum before being marked as permanently failed.

### Vector Index Management

Administrative endpoints for managing the HNSW vector index:

```bash
# Rebuild HNSW index from stored embeddings
POST /api/admin/management/database/{tenant}/{repo}/vector/rebuild

# Regenerate all embeddings (re-calls provider API)
POST /api/admin/management/database/{tenant}/{repo}/vector/regenerate

# Verify index integrity
POST /api/admin/management/database/{tenant}/{repo}/vector/verify

# Check index health
GET /api/admin/management/database/{tenant}/{repo}/vector/health

# Restore vector index from stored data (disaster recovery)
POST /api/admin/management/database/{tenant}/{repo}/vector/restore

# Vector metrics (monitoring)
GET /management/metrics/vector
```

These operations are also available via SQL:

```sql
REBUILD VECTOR INDEX;
VERIFY VECTOR INDEX;
SHOW VECTOR INDEX HEALTH;
REGENERATE EMBEDDINGS;
```

### Vector Metrics

The `/management/metrics/vector` endpoint exposes monitoring data for the HNSW vector index, including index size, cache hit rates, query latencies, and embedding job queue depth.

## HNSW Vector Search

The `raisin-hnsw` crate provides fast approximate nearest neighbor (ANN) search using the Hierarchical Navigable Small World (HNSW) algorithm.

### Key Properties

- **Multiple distance metrics** -- Cosine, L2, InnerProduct, and Hamming
- **O(log n) search** -- approximate nearest neighbor in logarithmic time
- **Memory-bounded** -- uses Moka LRU cache to limit memory usage
- **Multi-tenant** -- separate indexes per tenant/repo/branch
- **Persistent** -- periodic snapshots to disk with dirty tracking
- **Crash-safe** -- graceful shutdown ensures all dirty indexes are saved
- **Configurable HNSW parameters** -- tune connectivity, build quality, and search accuracy
- **Vector quantization** -- F32, F16, and Int8 storage types for memory/accuracy trade-offs

### Usage

```rust
use raisin_hnsw::HnswIndexingEngine;
use std::path::PathBuf;

// Create engine with 2GB cache and 1536-dimensional vectors (OpenAI)
let engine = HnswIndexingEngine::new(
    PathBuf::from("./.data/hnsw"),
    2 * 1024 * 1024 * 1024, // 2 GB cache
    1536,                    // embedding dimensions
)?;

// Start periodic snapshot task (saves every 60 seconds)
let snapshot_handle = engine.start_snapshot_task();

// Add an embedding
engine.add_embedding(
    "tenant1", "repo1", "main", "workspace1",
    "node-123", 42, // revision
    embedding_vector,
)?;

// Search for similar vectors
let results = engine.search(
    "tenant1", "repo1", "main", "workspace1",
    &query_vector,
    10, // top-k results
)?;

// Graceful shutdown
engine.shutdown().await?;
snapshot_handle.abort();
```

### Search Configuration

```rust
use raisin_hnsw::{SearchRequest, SearchMode, ScoringConfig};

let request = SearchRequest {
    query_vector: vec![0.1, 0.2, /* ... */],
    k: 10,
    mode: SearchMode::Documents, // or SearchMode::Chunks for multi-chunk results
    workspace_filter: None,      // None = search all workspaces
    // ...
};
```

`SearchMode` determines how multi-chunk documents are handled:
- `SearchMode::Documents` (default) -- deduplicates by source document, returning best chunk per document
- `SearchMode::Chunks` -- returns all matching chunks ranked by similarity

The `ScoringConfig` controls chunk-aware ranking with `position_decay` (earlier chunks score higher) and `first_chunk_boost`.

### HNSW Parameter Tuning

The HNSW index supports three tuning parameters that control the trade-off between index quality, build speed, and search accuracy:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `connectivity` (M) | 16 | Max edges per node. Higher values improve recall but increase memory usage |
| `expansion_add` (ef_construction) | 200 | Search width during index construction. Higher values improve index quality but slow down inserts |
| `expansion_search` (ef_search) | 100 | Search width during queries. Higher values improve recall at the cost of query latency |

Configure via the embedding config:

```sql
ALTER EMBEDDING CONFIG
  SET HNSW_CONNECTIVITY = 32
  SET HNSW_EXPANSION_ADD = 400
  SET HNSW_EXPANSION_SEARCH = 200;
```

### Vector Quantization

Vector quantization reduces memory usage by storing vectors in lower-precision formats:

| Type | Bytes per Dimension | Memory Savings | Notes |
|------|---------------------|----------------|-------|
| `F32` | 4 | Baseline (default) | Full precision |
| `F16` | 2 | 50% | Minimal accuracy loss for most use cases |
| `Int8` | 1 | 75% | Best for large indexes where memory is constrained |

### Distance Metrics

RaisinDB supports four distance metrics for vector search:

| Metric | SQL Operator | Description |
|--------|-------------|-------------|
| Cosine | `<=>` | Best for normalized embeddings (default) |
| L2 (Euclidean) | `<->` | Euclidean distance |
| InnerProduct | `<#>` | Negative dot product |
| Hamming | -- | Bitwise distance for binary vectors |

Configure the distance metric per tenant:

```sql
ALTER EMBEDDING CONFIG SET DISTANCE_METRIC = 'Cosine';
```

### Distance Interpretation

For cosine distance (1 - cosine similarity):

| Distance | Cosine Similarity | Interpretation |
|----------|-------------------|----------------|
| 0.0 | 1.0 | Identical vectors |
| 0.2 - 0.4 | 0.8 - 0.6 | Semantically similar |
| 0.4 - 0.6 | 0.6 - 0.4 | Weakly related |
| > 0.6 | < 0.4 | Not related |

### Configurable Max Distance

By default, vector search results are filtered to exclude vectors beyond a maximum distance threshold. This threshold is configurable per tenant:

```sql
-- Set the default max distance threshold (default: 0.6)
ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE = '0.5';
```

You can also filter by distance directly in SQL WHERE clauses:

```sql
-- Only return results within a specific distance
SELECT id, name, embedding <=> EMBEDDING('query') AS distance
FROM 'default'
WHERE embedding <=> EMBEDDING('query') < 0.3
ORDER BY distance
LIMIT 10
```

### Vector Search via SQL

Vector search is fully integrated into the SQL engine. The query planner automatically detects `ORDER BY vector_distance LIMIT k` patterns and uses the HNSW index for efficient approximate nearest neighbor search.

**Using the EMBEDDING() function** (generates a query vector from text at runtime):

```sql
-- Find 10 most similar articles to a text query
SELECT id, name, properties,
       embedding <=> EMBEDDING('machine learning tutorials') AS similarity
FROM 'default'
ORDER BY similarity
LIMIT 10
```

**Vector distance operators:**

| Operator | Function | Description |
|----------|----------|-------------|
| `<=>` | `VECTOR_COSINE_DISTANCE()` | Cosine distance (best for normalized embeddings) |
| `<->` | `VECTOR_L2_DISTANCE()` | Euclidean (L2) distance |
| `<#>` | `VECTOR_INNER_PRODUCT()` | Inner product distance |

**Combined with other filters:**

```sql
-- Semantic search within a specific path
SELECT id, name, embedding <=> EMBEDDING('rust database') AS sim
FROM 'default'
WHERE DESCENDANT_OF('/blog')
ORDER BY sim
LIMIT 10

-- Full-text search with vector re-ranking
SELECT id, name
FROM 'default'
WHERE FULLTEXT_MATCH('database engine', 'english')
ORDER BY embedding <=> EMBEDDING('database engine')
LIMIT 5
```

### Hybrid Search (HYBRID_SEARCH Table Function)

The `HYBRID_SEARCH` table function combines full-text search and vector search using Reciprocal Rank Fusion (RRF) to produce a single ranked result set:

```sql
-- Hybrid search combining fulltext + vector with RRF ranking
SELECT * FROM HYBRID_SEARCH('machine learning tutorials', 10)

-- With additional filtering
SELECT id, name, properties, score
FROM HYBRID_SEARCH('database optimization', 20)
WHERE node_type = 'myapp:Article'
```

Hybrid search runs both a full-text Tantivy query and a vector similarity search in parallel, then merges the results using RRF scoring. This typically produces better results than either search method alone, especially for queries where keyword matching and semantic similarity complement each other.

### EXPLAIN for Vector Queries

Use `EXPLAIN` to inspect vector query execution plans. The output shows `VectorScan` details including the distance metric, index parameters, and number of candidates:

```sql
EXPLAIN SELECT id, name, embedding <=> EMBEDDING('query') AS distance
FROM 'default'
ORDER BY distance
LIMIT 10
```

### Branch Operations

HNSW indexes support Git-like branch semantics -- when you create a new branch, the vector index can be copied efficiently for the new branch context.
