# Translation System: Vector Embedding Strategy

## Overview

This document describes the vector embedding strategy for the RaisinDB translation system. The key principle is: **vector embeddings are only created for base language content, not for translations**.

## Design Decision: Base Language Only

### Rationale

Vector embeddings represent semantic meaning in a high-dimensional space, enabling similarity search and semantic retrieval. When implementing multilingual content, we must decide:

1. **Option A: Embed all translations** - Create separate embeddings for each locale
2. **Option B: Embed base language only** - Single embedding per node (chosen)

We chose Option B for the following reasons:

### 1. Semantic Consistency

- Translations represent the **same semantic content** in different languages
- Creating separate embeddings would fragment semantically identical content
- A single embedding represents the core semantic meaning across all locales

### 2. Storage Efficiency

- One embedding per node vs. N embeddings per node (N = number of locales)
- For a node translated into 10 languages: 1 embedding vs. 10 embeddings
- Significant storage savings at scale

### 3. Query Simplicity

- Users search in their preferred language
- System translates query embedding to base language embedding space
- Results are then translated to user's locale for display
- Single index to search vs. multiple per-locale indexes

### 4. Consistency Across Locales

- All locales see the same search results (semantically)
- Results vary only in the displayed translation, not in relevance
- Avoids "translation drift" where embeddings diverge from base meaning

### 5. Reduced Indexing Overhead

- Translation updates don't trigger re-embedding
- Only base content changes require re-embedding
- Faster translation workflows

## Implementation Strategy

### Embedding Generation

```rust
// Only trigger embedding jobs for base language updates
if is_base_language_update(node, base_language) {
    emit_embedding_job(node_id, base_language);
}

// Translation updates do NOT trigger embedding jobs
if is_translation_update(node, locale) {
    // No embedding job needed
}
```

### Search Flow

1. **Query Input**: User enters search query in their locale (e.g., French)
2. **Query Embedding**: Generate embedding for query in user's locale
3. **Cross-Lingual Search**: Use multilingual embedding model (supports 100+ languages)
4. **Result Retrieval**: Find semantically similar base content
5. **Translation Resolution**: Apply locale fallback chain to translate results
6. **Display**: Return results in user's locale

### Multilingual Embedding Models

Modern embedding models (e.g., multilingual-e5, mBERT, LaBSE) are trained to map semantically similar text across languages to nearby points in embedding space.

**Example:**
```
Query (French): "recette de gâteau au chocolat"
-> Embedding: [0.23, -0.45, 0.67, ...]

Base Content (English): "chocolate cake recipe"
-> Embedding: [0.25, -0.43, 0.65, ...]

Cosine Similarity: 0.92 (high similarity despite different languages)
```

## Edge Cases and Considerations

### 1. Translation-Only Changes

**Scenario**: Translation updated but base content unchanged

**Behavior**:
- Embedding remains unchanged
- Search results unaffected
- Translation changes reflected in display only

**Example**:
```
Base (EN): "Welcome to our platform"
Embedding: [...]

Translation (FR): "Bienvenue sur notre plateforme"
-> No new embedding created
```

### 2. Base Content Changes

**Scenario**: Base content updated, triggering re-embedding

**Behavior**:
- New embedding generated
- Search results may change
- All translations continue to use new embedding

**Example**:
```
Before:
Base (EN): "Welcome to our platform"
Embedding: [0.1, 0.2, ...]

After:
Base (EN): "Welcome to our improved platform"
Embedding: [0.1, 0.3, ...] <- Re-generated
```

### 3. Translation Divergence

**Risk**: Translation deviates significantly from base meaning

**Mitigation**:
- Content review workflows
- Translation quality checks
- Semantic similarity validation (optional)

**Example of Problematic Divergence**:
```
Base (EN): "chocolate cake recipe" (food content)
Translation (FR): "recette de voiture" (car recipe - wrong!)

The embedding will still match chocolate cake queries,
but the displayed French text is incorrect.
```

**Solution**: Translation validation tools

### 4. Locale-Specific Content

**Scenario**: Content that should only appear in certain locales

**Solution**: Use `LocaleOverlay::Hidden` to hide in other locales

```rust
// Hide node in German locale
service.hide_node(
    tenant_id, repo_id, branch, workspace,
    node_id, &LocaleCode::parse("de")?,
    actor, Some("Content not applicable in German market"),
    revision
).await?;
```

## Vector Database Integration

### Indexing Strategy

```
Vector Index Structure:
{
  "node_id": "node-123",
  "embedding": [0.23, -0.45, ...],  // Base language embedding
  "base_language": "en",
  "metadata": {
    "node_type": "raisin:page",
    "path": "/products/item-1",
    "tenant_id": "tenant-1",
    "repo_id": "repo-1"
  }
}
```

### Query Strategy

```rust
// 1. Generate query embedding (in user's locale)
let query_embedding = embedding_model.embed(&query_text, user_locale);

// 2. Search vector index
let results = vector_db.similarity_search(
    query_embedding,
    limit: 10,
    filters: { tenant_id, repo_id }
);

// 3. Resolve translations
for result in results {
    let node = storage.get_node(result.node_id).await?;
    let translated_node = translation_resolver.resolve_node(
        tenant_id, repo_id, branch, workspace,
        node, &user_locale, current_revision
    ).await?;

    // Display translated node
}
```

## Fulltext Search Strategy

Fulltext search (Tantivy) follows a **per-locale indexing** strategy:

### Why Different from Vector Embeddings?

1. **Lexical vs. Semantic**: Fulltext is lexical (exact word matching), vectors are semantic
2. **Query Language Matters**: Users search with exact words in their language
3. **Tokenization**: Language-specific tokenizers improve accuracy
4. **Index Size**: Fulltext indexes are smaller than vector indexes

### Per-Locale Fulltext Indexes

```
Indexes:
- tenant-1_repo-1_en (base language)
- tenant-1_repo-1_fr (French translations)
- tenant-1_repo-1_de (German translations)
- tenant-1_repo-1_es (Spanish translations)
```

### Indexing Trigger

```rust
// Base content update -> Re-index base language
if is_base_language_update(node, base_language) {
    emit_fulltext_job(node_id, base_language);
}

// Translation update -> Re-index that locale
if is_translation_update(node, locale) {
    emit_fulltext_job(node_id, locale);
}
```

## Hybrid Search Strategy

Combining vector and fulltext search provides best of both worlds:

```rust
// Hybrid search example
async fn hybrid_search(
    query: String,
    locale: LocaleCode,
    tenant_id: &str,
    repo_id: &str
) -> Result<Vec<SearchResult>> {
    // 1. Semantic search (vector, base language)
    let vector_results = vector_search(query.clone(), locale).await?;

    // 2. Lexical search (fulltext, user's locale)
    let fulltext_results = fulltext_search(query, locale, tenant_id, repo_id).await?;

    // 3. Reciprocal Rank Fusion (RRF) to combine results
    let combined = reciprocal_rank_fusion(vector_results, fulltext_results);

    // 4. Translate to user's locale
    let translated = resolve_translations(combined, locale).await?;

    Ok(translated)
}
```

## Performance Characteristics

### Vector Embeddings

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| Index Size | O(N) | N = number of nodes |
| Query Latency | O(log N) | With HNSW index |
| Update Cost | O(1) per embedding | Only on base content changes |

### Fulltext Indexes

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| Index Size | O(N × L) | N = nodes, L = locales |
| Query Latency | O(log N) | Per-locale query |
| Update Cost | O(1) per locale | On translation updates |

## Migration Strategy

### Existing Deployments

For deployments adding translation support:

1. **One-Time Re-Embedding**: Not required (existing embeddings are base language)
2. **Fulltext Re-Indexing**: Required for per-locale indexes
3. **Zero Downtime**: Gradual rollout with feature flag

### Process

```bash
# 1. Create per-locale fulltext indexes
raisindb-admin create-locale-indexes --repo=repo-1

# 2. Index existing content in all locales
raisindb-admin reindex-translations --repo=repo-1

# 3. Enable translation features
raisindb-admin set-config --repo=repo-1 --translations=enabled
```

## Future Considerations

### Potential Enhancements

1. **Locale-Specific Embeddings** (Optional): For specific use cases
2. **Translation Quality Scoring**: Semantic similarity between base and translation
3. **Cross-Lingual Query Expansion**: Expand queries across locales
4. **Locale-Specific Ranking**: Adjust relevance by locale

### Research Areas

- Multilingual embedding model improvements
- Zero-shot cross-lingual transfer
- Semantic consistency validation
- Translation-aware ranking

## Summary

The RaisinDB translation system uses:

- **Vector Embeddings**: Base language only (1 per node)
- **Fulltext Indexes**: Per-locale (N per node, where N = number of locales)
- **Multilingual Models**: Enable cross-lingual semantic search
- **Translation Resolution**: Locale fallback chains for display

This strategy balances:
- Storage efficiency (single embedding)
- Search accuracy (multilingual models)
- Query performance (single index)
- Consistency (same results across locales)

---

**Last Updated**: 2025-10-27
**Version**: 1.0
**Status**: Implemented in Phase 1-2
