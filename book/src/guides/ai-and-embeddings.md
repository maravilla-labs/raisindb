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
| AWS Bedrock | Claude, Nova, Llama | Via AWS credentials |
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

### Embedding Storage

Embeddings are stored per-node and can be generated by different providers:

```rust
use raisin_embeddings::{EmbeddingStorage, EmbeddingData};

// Store an embedding for a node
let embedding = EmbeddingData {
    node_id: "node-123".to_string(),
    vector: vec![0.1, 0.2, 0.3, /* ... 1536 dimensions for OpenAI */],
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

Embedding generation runs asynchronously through the job system:

```rust
use raisin_embeddings::{EmbeddingJob, EmbeddingJobKind};

let job = EmbeddingJob {
    node_id: "node-123".to_string(),
    kind: EmbeddingJobKind::Generate,
    // ...
};
```

### Provider Abstraction

The `EmbeddingProvider` trait abstracts over different embedding APIs:

```rust
use raisin_embeddings::create_provider;

let provider = create_provider(&tenant_config).await?;
let vectors = provider.embed(vec!["Hello world".to_string()]).await?;
```

## HNSW Vector Search

The `raisin-hnsw` crate provides fast approximate nearest neighbor (ANN) search using the Hierarchical Navigable Small World (HNSW) algorithm.

### Key Properties

- **Cosine distance** -- optimized for normalized embeddings (OpenAI embeddings are pre-normalized)
- **O(log n) search** -- approximate nearest neighbor in logarithmic time
- **Memory-bounded** -- uses Moka LRU cache to limit memory usage
- **Multi-tenant** -- separate indexes per tenant/repo/branch
- **Persistent** -- periodic snapshots to disk with dirty tracking
- **Crash-safe** -- graceful shutdown ensures all dirty indexes are saved

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

### Distance Interpretation

Since HNSW uses cosine distance (1 - cosine similarity):

| Distance | Cosine Similarity | Interpretation |
|----------|-------------------|----------------|
| 0.0 | 1.0 | Identical vectors |
| 0.2 - 0.4 | 0.8 - 0.6 | Semantically similar |
| 0.4 - 0.6 | 0.6 - 0.4 | Weakly related |
| > 0.6 | < 0.4 | Not related |

### Vector Search via SQL

Vector search is integrated into the SQL engine. You can search for similar content using SQL:

```sql
SELECT id, name, __distance
FROM 'default'
WHERE VECTOR_SEARCH(embedding, $1, 10)
ORDER BY __distance ASC
```

### Branch Operations

HNSW indexes support Git-like branch semantics -- when you create a new branch, the vector index can be copied efficiently for the new branch context.
