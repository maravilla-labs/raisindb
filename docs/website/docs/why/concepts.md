---
sidebar_position: 2
---

# Core Concepts

RaisinDB is a multi-model database that combines document storage, graph relationships, vector search, and full-text indexing with git-like version control.

## Mental Model: Relational Database Concepts

RaisinDB uses a familiar mental model for developers coming from relational databases:

### Repository = Database
Repositories are the primary isolation boundary, analogous to a database:
- Each repository has its own data, branches, revisions, and configuration
- NodeType definitions (schemas) are shared at the repository level
- Use repositories to separate tenants/projects/products
- **Analogy:** `repository ≈ database`

### Workspace = Table
Workspaces are like tables within a database - they organize and separate data:
- Each workspace contains a collection of nodes
- Workspaces share the same NodeType definitions (schemas) within a repository
- Use workspaces to separate environments, content types, or logical groupings
- **SQL queries operate on workspaces** - the workspace name acts as the table name
- **Analogy:** `workspace ≈ table`

### NodeType = Schema
NodeTypes define the structure and validation rules for nodes:
- Like table schemas in SQL, they define columns (properties), types, and constraints
- Shared across all workspaces within a repository
- Provide type safety and validation
- Control which properties are indexed for SQL and full-text search
- **Analogy:** `NodeType ≈ table schema/DDL definition`

**Example mapping:**
```
Traditional SQL:          RaisinDB:
├── Database: shop       ├── Repository: shop
│   ├── Table: products  │   ├── Workspace: products
│   ├── Table: orders    │   ├── Workspace: orders
│   └── Schema: Product  │   └── NodeType: shop:Product
```

## Data Models

RaisinDB is a **multi-model database** supporting four complementary data models:

### 1. Document Model (Hierarchical)
Organize data in tree structures with parent-child relationships:

```
Repository Root
├── Folder (NodeType: raisin:Folder)
│   ├── Document (NodeType: custom:Article)
│   │   └── Asset (NodeType: raisin:Asset)
│   └── Subfolder (NodeType: raisin:Folder)
└── Standalone Document (NodeType: custom:Page)
```

**Benefits:**
- Natural hierarchical organization
- Path-based queries with `PATH_STARTS_WITH()`
- Efficient tree traversal
- Breadcrumb navigation

### 2. Graph Model (Relationships)
Create bidirectional relationships between any nodes:

```
User --[AUTHORED]--> Article
User --[MEMBER_OF]--> Organization
Article --[CATEGORIZED_AS]--> Category
```

**Query with SQL:**
```sql
-- Find all articles by a user
SELECT n.* FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED') AS e
JOIN nodes n ON n.id = e.dst_id;
```

**Benefits:**
- Many-to-many relationships
- Bidirectional navigation
- Complex relationship queries
- Social networks, knowledge graphs

### 3. Vector Model (Embeddings)
Store and search vector embeddings for semantic similarity:

```sql
-- Find similar articles
SELECT n.*, knn.distance
FROM KNN(:query_embedding, 20) AS knn
JOIN nodes n ON n.id = knn.node_id
ORDER BY knn.distance;
```

**Benefits:**
- Semantic search
- Recommendation systems
- AI/ML integration
- Content discovery

### 4. Full-Text Search
Blazing-fast text search powered by Tantivy:

```
Query: "rust performance optimization"
Results ranked by BM25 relevance
```

**Features:**
- 20+ language stemming
- Fuzzy matching
- Wildcard queries
- Boolean operators

## Nodes and NodeTypes

### Nodes
Nodes are individual data records that contain:
- **Properties**: JSONB key-value pairs holding actual data
- **Tree Relationships**: Parent-child hierarchical connections
- **Graph Relationships**: Named edges to other nodes
- **Embeddings**: Optional vector embeddings for similarity search
- **Metadata**: Version information, audit trails, timestamps

### NodeTypes (Schema Definitions)
NodeTypes are YAML-based schema definitions shared at the repository level:

```yaml
name: blog:Article
description: A blog post or article
properties:
  - name: title
    type: String
    required: true
    indexed_for_sql: true        # Enable SQL filtering
    fulltext_indexed: true        # Enable full-text search
  - name: content
    type: Text
    required: true
    fulltext_indexed: true
  - name: author
    type: Reference
    indexed_for_sql: true
  - name: tags
    type: Array
    indexed_for_sql: true
allowed_children: ["raisin:Asset"]
versionable: true
publishable: true
```

**Key features:**
- Define structure, types, validation, and constraints
- Control SQL and full-text indexing per property
- Apply uniformly across all workspaces in the repository
- Managed via `/api/management/{repo}/{branch}/nodetypes`

**Property Indexing:**
- `indexed_for_sql: true` - Enables efficient SQL filtering on this property
- `fulltext_indexed: true` - Includes this property in full-text search index
- Without these flags, properties are stored but not indexed for search

## Querying Data

RaisinDB provides multiple query interfaces:

### RaisinSQL (PostgreSQL-Compatible)
Query workspaces using SQL syntax:

```sql
-- Query a specific workspace (like a table)
SELECT id, name, properties ->> 'title' AS title
FROM workspace_name.nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC;

-- Hierarchical queries
SELECT * FROM workspace_name.nodes
WHERE DEPTH(path) = 3;

-- Graph traversal
SELECT n.* FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED') AS e
JOIN workspace_name.nodes n ON n.id = e.dst_id;

-- Vector similarity
SELECT n.*, knn.distance
FROM KNN(:embedding, 20) AS knn
JOIN workspace_name.nodes n ON n.id = knn.node_id;
```

[Learn more about RaisinSQL →](/docs/access/sql/raisinsql)

### Query DSL (JSON-Based)
Programmatic queries via REST API:

```json
{
  "and": [
    { "field": { "nodeType": { "eq": "blog:Article" } } },
    { "field": { "path": { "like": "/content/blog/" } } }
  ],
  "orderBy": { "created_at": "desc" },
  "limit": 10
}
```

### Full-Text Search API
Search across indexed properties:

```http
POST /api/fulltext/search
{
  "workspace_id": "main",
  "query": "rust performance",
  "limit": 20
}
```

[Learn more about querying →](/docs/access/sql/overview)

## Version Control (Git-like Workflows)

RaisinDB provides git-like version control at the repository level:

### Commits
- **Atomic changes** - all modifications in a commit succeed or fail together
- **Commit messages** - describe what changed and why
- **Author tracking** - know who made each change
- **Timestamps** - when changes occurred

### Branches and Tags
- **Branches** track head revisions for parallel development
- **Tags** point to immutable revisions for releases
- **Merging** combines changes from different branches
- The HTTP API addresses content via `/head/{ws}` or `/rev/{revision}/{ws}`

### Time-Travel Queries
Query data as it existed at any point in history:

```sql
-- Query current state
SELECT * FROM main.nodes WHERE path = '/article-1';

-- Query at specific revision
SELECT * FROM main.nodes@revision_123 WHERE path = '/article-1';
```

## Multi-tenancy and Registry

Tenants and deployments are managed via registry endpoints:
- Repositories are associated with tenants
- Isolation is per repository
- NodeTypes can be initialized per tenant/deployment but are stored repository-first

## Schema-driven Development

Define your data structure with YAML schemas to enable:
- **Type safety** - catch errors early
- **Documentation** - schemas serve as API documentation
- **Validation** - automatic data validation
- **Evolution** - version your schemas as requirements change
- **Indexing control** - specify which properties to index for SQL and full-text search

**Important:** Properties must be marked with `indexed_for_sql: true` or `fulltext_indexed: true` in the NodeType definition to be searchable via SQL queries or full-text search.

## Multilingual Content (Translations)

RaisinDB provides first-class support for multilingual content through its built-in translation system. Unlike traditional approaches that treat i18n as an application concern, RaisinDB makes translations a core database feature.

### Why Built-in Translations?

In a multi-model, version-controlled database like RaisinDB, translations need deep integration with the core data models:

#### 1. Structural Consistency Across Languages
When content has complex relationships—hierarchical trees, graph edges, vector embeddings—you need the structure to remain consistent while text varies by language. RaisinDB ensures that:
- The document tree hierarchy is identical across all locales
- Graph relationships remain consistent (a "User AUTHORED Article" relationship exists regardless of language)
- Vector embeddings can be locale-specific without duplicating node structure
- Metadata (creation dates, authors, IDs) stays unified

#### 2. Atomic Version Control
Every translation change creates a new revision, just like content changes. This enables:
- **Time-travel queries**: "Show me the German translation as it existed last month"
- **Translation history**: Track who translated what and when
- **Rollback capabilities**: Revert bad translations without affecting base content
- **Branching workflows**: Localization teams can work on translation branches

```sql
-- Query German translation at specific revision
SELECT * FROM articles@revision_500 WHERE path = '/blog/intro' LOCALE 'de';
```

#### 3. Multi-Model Query Integration
Translations work seamlessly across all four data models:

**Document Model:**
```sql
-- Hierarchical query with German localization
SELECT * FROM content
WHERE PATH_STARTS_WITH(path, '/blog/')
  AND DEPTH(path) = 2
LOCALE 'de';
```

**Graph Model:**
```sql
-- Relationship traversal with French localization
SELECT n.properties ->> 'title' AS title
FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED') AS e
JOIN content n ON n.id = e.dst_id
LOCALE 'fr';
```

**Vector Search:**
```sql
-- Semantic similarity with locale-specific embeddings
SELECT n.*, knn.distance
FROM KNN(:german_query_embedding, 20) AS knn
JOIN content n ON n.id = knn.node_id
LOCALE 'de';
```

**Full-Text Search:**
```
Query: "Einführung in Rust" (German)
-- Searches German translations with German stemming
-- Falls back to English if no German translation exists
```

#### 4. Fallback Chain Resolution
RaisinDB automatically traverses configured fallback chains:

```
Request: Swiss German (de-CH)
Fallback chain: de-CH → de → en
Result: Best available translation
```

This prevents broken content when translations are incomplete—users always see something meaningful.

#### 5. Schema-Controlled Translatability
NodeType definitions declare which properties can be translated:

```yaml
properties:
  - name: title
    type: String
    translatable: true      # User-visible text
  - name: author
    type: Reference
    translatable: false     # Same across languages
  - name: views
    type: Number
    translatable: false     # Analytics data
```

This prevents accidentally translating metadata, references, or system fields.

### Translation Storage Model

Translations are stored as **overlays** that sit on top of base content:

```
Base Node (default language: en):
{
  "id": "article-1",
  "path": "/blog/intro",
  "properties": {
    "title": "Introduction to RaisinDB",
    "author": "user-123",
    "views": 1500
  }
}

German Overlay (revision 42):
{
  "node_id": "article-1",
  "locale": "de",
  "revision": 42,
  "translations": {
    "/title": "Einführung in RaisinDB"
    // author and views not translated
  }
}
```

When you query with `?lang=de`, RaisinDB:
1. Fetches the base node
2. Applies the German overlay (if it exists)
3. Returns merged content with locale-specific title

### Hidden Nodes Per Locale

Mark nodes as hidden in specific markets without deleting them:

```bash
# Hide US-only content in EU
POST /api/repository/shop/main/head/promos/blackfriday/raisin:cmd/hide-in-locale
{ "locale": "de" }

# Content automatically filtered for German users
GET /api/repository/shop/main/head/promos/?lang=de
```

Use cases:
- Market-specific features (payments, regulations)
- Gradual rollout (hide until translation complete)
- A/B testing per region

### Git-like Translation Workflows

Because translations are versioned, localization teams can use familiar git workflows:

```bash
# Create localization branch
POST /api/management/repositories/default/shop/branches
{ "name": "i18n/german", "from_revision": 100 }

# Translators work on their branch
POST /api/repository/shop/i18n~german/head/content/.../raisin:cmd/translate
{ "locale": "de", "translations": {...} }

# Review and merge when ready
POST /api/management/repositories/default/shop/branches/main/merge
{ "from_branch": "i18n/german" }
```

### Why Not Use Application-Level i18n?

Traditional approaches store translations in separate tables or services, which breaks down in a multi-model database:

| Concern | App-Level i18n | RaisinDB Built-in |
|---------|----------------|-------------------|
| **Structure** | Duplicate node hierarchies per language | Single structure, localized properties |
| **Relationships** | Complex to maintain graph edges | Relationships work across locales |
| **Versioning** | Separate translation history | Unified version control |
| **Queries** | Join translated tables manually | SQL `LOCALE` clause |
| **Fallbacks** | Application logic | Database-enforced chains |
| **Time-travel** | Not supported | Query any locale at any revision |

RaisinDB's approach ensures that translations are:
- **Atomic**: Changes are versioned with the content
- **Consistent**: Structure identical across locales
- **Queryable**: SQL and API support built-in
- **Efficient**: No duplication of nodes or relationships

[Learn more about the Translation API →](/docs/access/rest/translations)

## Next Steps

- 🔍 [Learn about querying](/docs/access/sql/overview) - SQL, graph, vector, and full-text search
- 🌐 [Explore the Translation API](/docs/access/rest/translations) - Multilingual content management
- 🏗️ [Understand the architecture](/docs/why/architecture) - system design and components
- 📝 [Define your first NodeType](/docs/model/nodetypes/overview) - schema definitions
- 🔧 [Explore the REST API](/docs/access/rest/overview) - HTTP endpoints
