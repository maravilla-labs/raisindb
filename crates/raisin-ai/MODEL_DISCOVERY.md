# Dynamic Model Discovery

This document describes the dynamic model discovery feature in the `raisin-ai` crate, which allows applications to fetch and cache available AI models from different providers at runtime.

## Overview

Instead of using hardcoded model lists, the `raisin-ai` crate now supports dynamic model discovery through the `list_available_models()` method on the `AIProviderTrait`. This provides several benefits:

- **Always Up-to-Date**: Automatically discover new models as providers release them
- **User-Specific**: OpenAI fine-tuned models and Ollama local models are automatically discovered
- **Rich Metadata**: Get detailed information about each model's capabilities, context window, and features
- **Performance**: Built-in caching with TTL prevents excessive API calls

## Architecture

### ModelInfo Structure

Each model is represented by a `ModelInfo` struct containing:

```rust
pub struct ModelInfo {
    pub id: String,                           // Unique model identifier
    pub name: String,                         // Human-readable name
    pub capabilities: ModelCapabilities,       // What the model can do
    pub context_window: Option<u32>,          // Context size in tokens
    pub max_output_tokens: Option<u32>,       // Maximum output size
    pub available: bool,                       // Whether currently available
    pub metadata: Option<serde_json::Value>,  // Provider-specific metadata
}
```

### ModelCapabilities

Capabilities indicate what a model supports:

```rust
pub struct ModelCapabilities {
    pub chat: bool,         // Supports chat/conversation
    pub embeddings: bool,   // Supports embeddings
    pub vision: bool,       // Supports vision/image inputs
    pub tools: bool,        // Supports tool/function calling
    pub streaming: bool,    // Supports streaming responses
}
```

### Caching System

The `ModelCache` provides in-memory caching with configurable TTL:

- **OpenAI**: 1 hour TTL (models don't change frequently)
- **Anthropic**: 1 hour TTL (static list of known models)
- **Ollama**: 5 minutes TTL (local models can be added/removed frequently)

## Provider-Specific Implementation

### OpenAI

OpenAI provides a `/v1/models` API endpoint that returns all available models:

```rust
let provider = OpenAIProvider::new(api_key);
let models = provider.list_available_models().await?;

for model in models {
    println!("{}: context={}", model.id, model.context_window.unwrap_or(0));
}
```

**Features**:
- Fetches from `GET https://api.openai.com/v1/models`
- Includes fine-tuned models specific to your account
- Filters to relevant models (GPT, O1, embeddings)
- Estimates context window based on model ID
- Cached for 1 hour

**Example Response**:
```json
{
  "id": "gpt-4o",
  "name": "gpt-4o",
  "capabilities": {
    "chat": true,
    "tools": true,
    "vision": true,
    "streaming": true,
    "embeddings": false
  },
  "context_window": 128000,
  "metadata": {
    "owned_by": "openai",
    "created": 1234567890
  }
}
```

### Anthropic

Anthropic doesn't provide a models API, so we maintain a curated list of known models:

```rust
let provider = AnthropicProvider::new(api_key);
let models = provider.list_available_models().await?;

for model in models {
    if let Some(metadata) = &model.metadata {
        println!("{}: family={}", model.id, metadata["family"]);
    }
}
```

**Features**:
- Static list of known Claude models (updated with crate releases)
- Includes Claude 4.5, 3.5, and 3.0 families
- Detailed metadata (family, tier, release date)
- Cached for 1 hour
- Last updated: 2025-01

**Available Models**:
- Claude Opus 4.5 (200K context, 16K output)
- Claude Sonnet 4.5 (200K context, 8K output)
- Claude 3.5 Sonnet (200K context, 8K output)
- Claude 3.5 Haiku (200K context, 8K output)
- Claude 3 Opus/Sonnet/Haiku (200K context, 4K output)

### Ollama

Ollama provides a `/api/tags` endpoint that lists locally installed models:

```rust
let provider = OllamaProvider::new();
let models = provider.list_available_models().await?;

for model in models {
    println!("{}: size={} GB",
        model.id,
        model.metadata.unwrap()["size"].as_u64().unwrap() / 1_000_000_000
    );
}
```

**Features**:
- Fetches from `GET http://localhost:11434/api/tags`
- Shows exactly what's installed on your system
- Includes model size, digest, and modification time
- Detects vision models (llava, vision variants)
- Cached for 5 minutes (shorter TTL since models change)
- Returns helpful error if Ollama isn't running

**Example Response**:
```json
{
  "id": "llama3.3:latest",
  "name": "llama3.3:latest",
  "capabilities": {
    "chat": true,
    "tools": true,
    "vision": false,
    "streaming": true,
    "embeddings": false
  },
  "context_window": 4096,
  "metadata": {
    "size": 4661224992,
    "digest": "abc123...",
    "modified_at": "2025-01-15T10:30:00Z",
    "details": {
      "format": "gguf",
      "family": "llama",
      "parameter_size": "7B"
    }
  }
}
```

## Usage Examples

### Basic Model Listing

```rust
use raisin_ai::providers::openai::OpenAIProvider;
use raisin_ai::provider::AIProviderTrait;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OpenAIProvider::new("sk-...");

    let models = provider.list_available_models().await?;

    for model in models {
        println!("{}", model.id);
    }

    Ok(())
}
```

### Filter by Capability

```rust
// Find all models that support tools
let models = provider.list_available_models().await?;
let tool_models: Vec<_> = models
    .into_iter()
    .filter(|m| m.capabilities.tools)
    .collect();

println!("Models with tool support:");
for model in tool_models {
    println!("  - {}", model.id);
}
```

### Find Best Model for Task

```rust
// Find the model with the largest context window that supports vision
let models = provider.list_available_models().await?;
let best_vision_model = models
    .into_iter()
    .filter(|m| m.capabilities.vision)
    .max_by_key(|m| m.context_window.unwrap_or(0));

if let Some(model) = best_vision_model {
    println!("Best vision model: {} with {} token context",
        model.id,
        model.context_window.unwrap()
    );
}
```

### Admin UI Integration

```rust
// In your API handler
async fn list_provider_models(provider_id: &str) -> Result<Vec<ModelInfo>> {
    let provider = get_provider(provider_id)?;

    // This will use cache if available
    let models = provider.list_available_models().await?;

    Ok(models)
}

// Frontend can now populate dropdown with current models
// GET /api/admin/providers/openai/models
```

### Model Selection Helper

```rust
/// Helper to validate and recommend models
pub async fn recommend_model(
    provider: &impl AIProviderTrait,
    requirements: ModelRequirements,
) -> Result<ModelInfo> {
    let models = provider.list_available_models().await?;

    let suitable: Vec<_> = models
        .into_iter()
        .filter(|m| {
            if requirements.needs_tools && !m.capabilities.tools {
                return false;
            }
            if requirements.needs_vision && !m.capabilities.vision {
                return false;
            }
            if let Some(min_context) = requirements.min_context {
                if m.context_window.unwrap_or(0) < min_context {
                    return false;
                }
            }
            true
        })
        .collect();

    suitable
        .into_iter()
        .next()
        .ok_or_else(|| ProviderError::InvalidModel("No suitable model found".into()))
}
```

## Cache Management

### Manual Cache Control

```rust
use raisin_ai::ModelCache;

let cache = ModelCache::new();

// Get cached models
if let Some(models) = cache.get("openai").await {
    println!("Using cached models");
}

// Invalidate cache for a provider
cache.invalidate("openai").await;

// Clear all caches
cache.clear().await;

// Clean up expired entries
cache.cleanup().await;
```

### Custom TTL

```rust
use std::time::Duration;

let provider = OpenAIProvider::new(api_key);
// Cache is created with default TTL (1 hour)

// To use a custom cache, you'd need to modify the provider
// (currently not exposed in public API)
```

## Error Handling

The `list_available_models()` method returns `Result<Vec<ModelInfo>, ProviderError>`:

```rust
match provider.list_available_models().await {
    Ok(models) => {
        // Process models
    }
    Err(ProviderError::NetworkError(e)) => {
        eprintln!("Network error: {}", e);
    }
    Err(ProviderError::InvalidApiKey) => {
        eprintln!("Invalid API key");
    }
    Err(ProviderError::ProviderNotAvailable(e)) => {
        eprintln!("Provider not available: {}", e);
    }
    Err(e) => {
        eprintln!("Error: {}", e);
    }
}
```

## Testing

Run the example to see model discovery in action:

```bash
# Set API keys
export OPENAI_API_KEY=sk-...
export ANTHROPIC_API_KEY=sk-ant-...

# Run the example
cargo run --example list_models
```

## Future Enhancements

Potential improvements for future versions:

1. **Model Health Checks**: Verify model availability before returning
2. **Model Comparison**: Helper functions to compare model capabilities
3. **Cost Estimation**: Include pricing information in metadata
4. **Model Recommendations**: AI-powered model selection based on task
5. **Persistent Cache**: Optional file-based cache across restarts
6. **Webhooks**: Notify when new models are discovered
7. **Model Versioning**: Track model versions and deprecations

## Migration Guide

If you're using the old `available_models()` method:

```rust
// Old way (still works)
let model_ids = provider.available_models();

// New way (recommended)
let models = provider.list_available_models().await?;
let model_ids: Vec<String> = models.into_iter().map(|m| m.id).collect();
```

The old method is still available for backward compatibility but is deprecated.

## Performance Considerations

- **Cache Hit Rate**: With 1-hour TTL, expect >99% cache hit rate in production
- **API Latency**: OpenAI models endpoint: ~200-500ms, Ollama: <50ms
- **Memory Usage**: ~1KB per model in cache, negligible for typical deployments
- **Concurrent Requests**: Cache uses `RwLock` for thread-safe concurrent access

## Security Notes

- Model lists may reveal account-specific information (fine-tuned models)
- Don't expose raw model metadata to untrusted clients
- Cache is in-memory only, cleared on restart
- No sensitive information (API keys) stored in cache
