# Translation System: Fulltext Indexing Strategy

## Overview

This document describes how fulltext indexing (Tantivy) integrates with the translation system. Unlike vector embeddings (which use base language only), fulltext indexes are created per-locale to enable precise lexical matching in each language.

## Design: Per-Locale Fulltext Indexes

### Rationale

Fulltext search is **lexical** (word-based), not semantic. Users search with exact words in their language, requiring language-specific:

1. **Tokenization**: Different languages have different word boundaries
2. **Stemming**: Language-specific word normalization (running → run)
3. **Stop Words**: Common words to ignore vary by language
4. **Character Normalization**: Accents, case, special characters

### Index Structure

```
Index Naming Pattern:
{tenant_id}_{repo_id}_{locale}

Examples:
- tenant-1_repo-1_en  (English - base language)
- tenant-1_repo-1_fr  (French translations)
- tenant-1_repo-1_de  (German translations)
- tenant-1_repo-1_es  (Spanish translations)
```

## Job Emission Strategy

### When to Emit Fulltext Jobs

| Event | Base Content | Translation | Job Emitted |
|-------|-------------|-------------|-------------|
| Node Created | ✓ | - | Base language index |
| Node Updated | ✓ | - | Base language index |
| Node Deleted | ✓ | - | Base language index (delete) |
| Translation Created | - | ✓ | Locale-specific index |
| Translation Updated | - | ✓ | Locale-specific index |
| Translation Deleted | - | ✓ | Locale-specific index (delete) |
| Node Hidden in Locale | - | ✓ | Locale-specific index (delete) |

### Implementation Location

Fulltext indexing jobs should be emitted from the **TranslationService** and **BlockTranslationService** after successfully storing translations.

#### Pattern: After Store

```rust
// In TranslationService::update_translation()
pub async fn update_translation(...) -> Result<TranslationUpdateResult> {
    // 1. Store the translation
    self.repository
        .store_translation(tenant_id, repo_id, branch, workspace, node_id, locale, &overlay, &meta)
        .await?;

    // 2. Emit fulltext indexing job for this locale
    self.emit_fulltext_job(tenant_id, repo_id, branch, workspace, node_id, locale)
        .await?;

    Ok(TranslationUpdateResult { ... })
}
```

### Job Structure

```rust
use raisin_storage::{FullTextIndexJob, JobKind};

// Create fulltext indexing job
let job = FullTextIndexJob {
    tenant_id: tenant_id.to_string(),
    repo_id: repo_id.to_string(),
    branch: branch.to_string(),
    workspace: workspace.to_string(),
    node_id: node_id.to_string(),
    locale: locale.as_str().to_string(),  // Important: locale-specific!
    kind: JobKind::Index,  // or JobKind::Delete
    properties_to_index: None,  // or Some(vec![...]) for specific properties
    revision: Some(revision),
};

// Enqueue the job
job_store.enqueue_job(job).await?;
```

## Indexing Worker Behavior

### Index Document Structure

Per-locale Tantivy document:

```rust
{
  "node_id": "node-123",
  "locale": "fr",              // Important: locale marker
  "tenant_id": "tenant-1",
  "repo_id": "repo-1",
  "branch": "main",
  "workspace": "live",
  "path": "/content/article-1",
  "node_type": "raisin:page",

  // Indexed content (in French)
  "title": "Bienvenue sur notre plateforme",
  "content": "Ceci est un article en français...",
  "description": "Description de l'article",

  // Metadata
  "revision": 42,
  "updated_at": "2025-10-27T10:00:00Z"
}
```

### Worker Processing

```rust
async fn process_translation_job(job: FullTextIndexJob) -> Result<()> {
    // 1. Get the base node
    let node = storage.nodes().get(
        &job.tenant_id, &job.repo_id, &job.branch,
        &job.workspace, &job.node_id, None
    ).await?;

    // 2. Resolve translation for the specified locale
    let locale = LocaleCode::parse(&job.locale)?;
    let translated_node = translation_resolver.resolve_node(
        &job.tenant_id, &job.repo_id, &job.branch, &job.workspace,
        node, &locale, job.revision.unwrap_or(u64::MAX)
    ).await?;

    // 3. Handle Hidden nodes
    if translated_node.is_none() {
        // Node is hidden in this locale - remove from index
        tantivy_engine.delete_document(&job.tenant_id, &job.repo_id, &job.locale, &job.node_id).await?;
        return Ok(());
    }

    let translated_node = translated_node.unwrap();

    // 4. Extract properties to index
    let properties = extract_indexable_properties(&translated_node, indexing_policy)?;

    // 5. Index the document (with locale-specific analyzer)
    tantivy_engine.index_document(
        &job.tenant_id,
        &job.repo_id,
        &job.locale,  // Locale determines which index and analyzer
        &translated_node,
        properties
    ).await?;

    Ok(())
}
```

## Language Analyzers

### Tantivy Analyzer Configuration

```rust
use tantivy::tokenizer::*;

fn get_analyzer_for_locale(locale: &str) -> Box<dyn Tokenizer> {
    match locale {
        "en" => Box::new(
            TextAnalyzer::from(SimpleTokenizer)
                .filter(RemoveLongFilter::limit(40))
                .filter(LowerCaser)
                .filter(Stemmer::new(Language::English))
        ),
        "fr" => Box::new(
            TextAnalyzer::from(SimpleTokenizer)
                .filter(RemoveLongFilter::limit(40))
                .filter(LowerCaser)
                .filter(Stemmer::new(Language::French))
        ),
        "de" => Box::new(
            TextAnalyzer::from(SimpleTokenizer)
                .filter(RemoveLongFilter::limit(40))
                .filter(LowerCaser)
                .filter(Stemmer::new(Language::German))
        ),
        "es" => Box::new(
            TextAnalyzer::from(SimpleTokenizer)
                .filter(RemoveLongFilter::limit(40))
                .filter(LowerCaser)
                .filter(Stemmer::new(Language::Spanish))
        ),
        _ => Box::new(
            // Default analyzer for unsupported languages
            TextAnalyzer::from(SimpleTokenizer)
                .filter(RemoveLongFilter::limit(40))
                .filter(LowerCaser)
        ),
    }
}
```

## Query Strategy

### User Query Flow

```rust
async fn fulltext_search(
    query: String,
    locale: LocaleCode,
    tenant_id: &str,
    repo_id: &str,
    limit: usize
) -> Result<Vec<SearchResult>> {
    // 1. Determine which index to query
    let index_name = format!("{}_{}_{}",  tenant_id, repo_id, locale.as_str());

    // 2. Parse query with locale-specific analyzer
    let parsed_query = query_parser.parse_query_with_locale(&query, &locale)?;

    // 3. Search the locale-specific index
    let hits = tantivy_engine.search(
        &index_name,
        parsed_query,
        limit
    ).await?;

    // 4. Results are already in the correct locale
    Ok(hits)
}
```

### Multi-Locale Search

For searching across multiple locales:

```rust
async fn multi_locale_search(
    query: String,
    locales: Vec<LocaleCode>,
    tenant_id: &str,
    repo_id: &str,
    limit: usize
) -> Result<Vec<SearchResult>> {
    let mut all_results = Vec::new();

    // Search each locale index
    for locale in locales {
        let results = fulltext_search(
            query.clone(), locale, tenant_id, repo_id, limit
        ).await?;
        all_results.extend(results);
    }

    // Deduplicate by node_id, preferring user's primary locale
    let deduplicated = deduplicate_by_node_id(all_results);

    Ok(deduplicated)
}
```

## Integration with Translation Services

### TranslationService Extensions

Add fulltext job emission to translation services:

```rust
impl<R: TranslationRepository> TranslationService<R> {
    // New field
    job_store: Arc<dyn FullTextJobStore>,

    async fn emit_fulltext_job(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        locale: &LocaleCode,
    ) -> Result<()> {
        let job = FullTextIndexJob {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace: workspace.to_string(),
            node_id: node_id.to_string(),
            locale: locale.as_str().to_string(),
            kind: JobKind::Index,
            properties_to_index: None,
            revision: None,
        };

        self.job_store.enqueue_job(job).await?;
        Ok(())
    }

    // Updated method
    pub async fn update_translation(...) -> Result<TranslationUpdateResult> {
        // Store translation
        self.repository.store_translation(...).await?;

        // Emit indexing job
        self.emit_fulltext_job(tenant_id, repo_id, branch, workspace, node_id, locale).await?;

        Ok(TranslationUpdateResult { ... })
    }

    // Similar for batch_update, hide_node, unhide_node, delete_translation
}
```

### BlockTranslationService Extensions

```rust
impl<R: TranslationRepository> BlockTranslationService<R> {
    // Similar pattern for block translations
    // Emit jobs after updating block translations
}
```

## Performance Considerations

### Index Size

- **Per-Locale Cost**: Each locale adds an index
- **Typical Size**: 10-20% of source content size
- **Example**: 1GB base content, 5 locales = ~600MB total index size

### Query Performance

- **Single Locale**: O(log N) per query
- **Multi-Locale**: O(L × log N) where L = number of locales searched
- **Recommendation**: Search single locale by default, offer multi-locale as option

### Indexing Performance

- **Parallel Indexing**: Jobs can be processed in parallel across locales
- **Incremental Updates**: Only changed translations need re-indexing
- **Batch Operations**: Batch translation updates share revision

## Migration Strategy

### Adding Translation Support to Existing Deployments

```bash
# 1. Create per-locale indexes for all supported languages
raisindb-admin create-locale-indexes \
  --tenant=tenant-1 \
  --repo=repo-1 \
  --locales=en,fr,de,es

# 2. Re-index existing content (base language only initially)
raisindb-admin reindex-repo \
  --tenant=tenant-1 \
  --repo=repo-1 \
  --locale=en

# 3. As translations are added, they'll auto-index via job emission

# 4. (Optional) Bulk re-index if translations already exist
raisindb-admin reindex-translations \
  --tenant=tenant-1 \
  --repo=repo-1 \
  --all-locales
```

### Index Management

```bash
# Check index sizes
raisindb-admin index-stats --tenant=tenant-1 --repo=repo-1

# Optimize indexes (merge segments)
raisindb-admin optimize-indexes --tenant=tenant-1 --repo=repo-1

# Delete locale index
raisindb-admin delete-locale-index \
  --tenant=tenant-1 \
  --repo=repo-1 \
  --locale=de
```

## Testing Strategy

### Unit Tests

```rust
#[tokio::test]
async fn test_translation_emits_fulltext_job() {
    let mock_job_store = Arc::new(MockJobStore::new());
    let service = TranslationService::new(repo, mock_job_store.clone());

    service.update_translation(...).await.unwrap();

    // Verify job was emitted
    let jobs = mock_job_store.get_enqueued_jobs();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].locale, "fr");
    assert_eq!(jobs[0].kind, JobKind::Index);
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_translation_fulltext_search() {
    // 1. Create base content
    let node = create_test_node("Hello world");
    storage.nodes().put(node).await.unwrap();

    // 2. Add French translation
    service.update_translation(
        node_id, &LocaleCode::parse("fr")?,
        translations_fr
    ).await.unwrap();

    // 3. Wait for indexing job to process
    wait_for_indexing_jobs().await;

    // 4. Search in French
    let results = fulltext_search("bonjour", LocaleCode::parse("fr")?).await.unwrap();

    // 5. Verify French translation found
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].locale, "fr");
}
```

## Summary

The RaisinDB translation system uses **per-locale fulltext indexes**:

- ✅ Language-specific tokenization and stemming
- ✅ Accurate lexical matching in each language
- ✅ Jobs emitted from TranslationService after storing
- ✅ Worker resolves translations before indexing
- ✅ Hidden nodes removed from locale indexes
- ✅ Parallel indexing across locales

**Key Difference from Vector Embeddings**:
- Vector: Base language only (semantic search)
- Fulltext: Per-locale (lexical search)

---

**Last Updated**: 2025-10-27
**Version**: 1.0
**Status**: Design Document - Implementation in Phase 5
