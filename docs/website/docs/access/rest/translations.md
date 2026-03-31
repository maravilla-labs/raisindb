---
sidebar_position: 4
---

# Translation API

RaisinDB provides first-class support for multilingual content through its built-in translation system. Translations are stored as revision-aware overlays that allow you to maintain localized versions of your content without duplicating node structures.

## Why Built-in Translations?

While many databases treat i18n as an application concern, RaisinDB makes translations a core database feature for several compelling reasons:

### 1. Structural Integrity
Content structure (hierarchy, relationships, metadata) remains consistent across all languages. Only translatable properties differ by locale, ensuring your data model stays coherent.

### 2. Atomic Versioning
Translations are revision-tracked just like base content. You can:
- Query content as it existed in any language at any point in time
- Compare translation changes across revisions
- Revert or restore translations independently

### 3. SQL-Aware Localization
Translations work seamlessly with RaisinSQL queries:

```sql
-- Automatic locale resolution
SELECT * FROM articles WHERE title LIKE 'Einführung%' LOCALE 'de';

-- Fallback chain traversal
SELECT * FROM articles WHERE status = 'published' LOCALE 'de-CH';
-- Falls back: de-CH → de → en (configurable)
```

### 4. Git-like Workflows
Translations inherit all version control benefits:
- Branch translations separately for localization teams
- Merge translation updates independently of content changes
- Tag releases with complete multilingual snapshots

### 5. Multi-Model Consistency
Translations apply uniformly across all RaisinDB data models:
- **Document trees** - preserve hierarchy regardless of language
- **Graph relationships** - relationships remain consistent across locales
- **Vector embeddings** - generate locale-specific embeddings automatically
- **Full-text search** - search in user's preferred language with fallbacks

---

## Core Concepts

### Translation Overlays
Translations are stored as **overlays** on top of base content:

```
Base Content (default language):
{
  "id": "page-123",
  "path": "/products/widget",
  "properties": {
    "title": "Amazing Widget",
    "description": "The best widget ever"
  }
}

German Overlay (de):
{
  "node_id": "page-123",
  "locale": "de",
  "translations": {
    "/title": "Erstaunliches Widget",
    "/description": "Das beste Widget aller Zeiten"
  }
}
```

When you query with `?lang=de`, RaisinDB automatically applies the German overlay to return localized content.

### Locale Fallback Chains
Define fallback chains in repository configuration:

```json
{
  "default_language": "en",
  "supported_languages": ["en", "de", "de-CH", "fr"],
  "locale_fallback_chains": {
    "de-CH": ["de-CH", "de", "en"],
    "de-AT": ["de-AT", "de", "en"],
    "fr-CA": ["fr-CA", "fr", "en"]
  }
}
```

If a translation doesn't exist for a locale, RaisinDB traverses the fallback chain until it finds a value or reaches the default language.

### Translatable Properties
NodeType definitions control which properties can be translated:

```yaml
name: blog:Article
properties:
  - name: title
    type: String
    translatable: true        # Can be translated
  - name: author
    type: Reference
    translatable: false       # Same across all languages
  - name: publishDate
    type: Date
    translatable: false       # Metadata not translated
```

### Hidden Nodes
Mark nodes as hidden in specific locales without deleting them:

```json
// Hidden overlay for locale "de"
{
  "node_id": "promo-123",
  "locale": "de",
  "overlay": "Hidden"
}
```

Use cases:
- Market-specific content (hide US promotions in EU)
- Compliance (hide content not localized for certain regions)
- Gradual rollout (hide until translation complete)

---

## Translation Configuration

### Get Translation Configuration

Retrieve translation settings for a repository.

**Endpoint:**
```
GET /api/repositories/{repo}/translation-config
```

**Response:**
```json
{
  "default_language": "en",
  "supported_languages": ["en", "de", "fr", "es"],
  "locale_fallback_chains": {
    "de-CH": ["de-CH", "de", "en"],
    "fr-CA": ["fr-CA", "fr", "en"]
  }
}
```

### Update Translation Configuration

Configure supported languages and fallback chains.

**Endpoint:**
```
PUT /api/repositories/{repo}
```

**Body:**
```json
{
  "supported_languages": ["en", "de", "fr", "es", "ja"],
  "locale_fallback_chains": {
    "de-CH": ["de-CH", "de", "en"],
    "de-AT": ["de-AT", "de", "en"],
    "ja-JP": ["ja-JP", "ja", "en"]
  }
}
```

---

## Managing Translations

All translation operations use the `raisin:cmd` command pattern:

```
POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/{command}
```

### Create or Update Translation

Add or modify translations for a node in a specific locale.

**Command:** `translate`

**Endpoint:**
```
POST /api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/translate
```

**Body:**
```json
{
  "locale": "de",
  "translations": {
    "/title": "Erstaunliches Produkt",
    "/description": "Eine ausführliche Beschreibung"
  },
  "message": "Add German translation",
  "actor": "translator@example.com"
}
```

**Response:**
```json
{
  "node_id": "page-123",
  "locale": "de",
  "revision": 142,
  "timestamp": "2025-01-15T10:30:00Z"
}
```

**Notes:**
- Uses JSON Pointer syntax (`/property`) for translation keys
- Creates a new revision for each translation update
- Service layer manages revision creation automatically
- Translations only apply to properties marked `translatable: true`

### List Translations

Get all available translations for a node.

**Command:** `list-translations`

**Endpoint:**
```
POST /api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/list-translations
```

**Response:**
```json
{
  "node_id": "page-123",
  "locales": ["de", "fr", "es"]
}
```

### Delete Translation

Remove a translation, reverting to fallback chain.

**Command:** `delete-translation`

**Endpoint:**
```
POST /api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/delete-translation
```

**Body:**
```json
{
  "locale": "de",
  "message": "Remove outdated German translation",
  "actor": "admin@example.com"
}
```

### Hide Node in Locale

Hide a node for specific locales without deleting it.

**Command:** `hide-in-locale`

**Endpoint:**
```
POST /api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/hide-in-locale
```

**Body:**
```json
{
  "locale": "de",
  "message": "Hide US-only promotion in Germany",
  "actor": "content-manager@example.com"
}
```

**Response:**
```json
{
  "node_id": "promo-123",
  "locale": "de",
  "revision": 143,
  "timestamp": "2025-01-15T11:00:00Z"
}
```

### Unhide Node in Locale

Remove the hidden state, making the node visible again.

**Command:** `delete-translation` (removes Hidden overlay)

**Endpoint:**
```
POST /api/repository/{repo}/{branch}/head/{ws}/{*path}/raisin:cmd/delete-translation
```

**Body:**
```json
{
  "locale": "de",
  "message": "Unhide node in German market",
  "actor": "content-manager@example.com"
}
```

---

## Querying Localized Content

### HTTP API with Locale Parameter

Fetch nodes in a specific language using the `lang` query parameter:

```
GET /api/repository/{repo}/{branch}/head/{ws}/{*path}?lang=de
```

**Behavior:**
- Returns node with translations applied
- Follows fallback chain if translation missing
- Properties without translations show default language value

**Example Response:**
```json
{
  "id": "page-123",
  "path": "/products/widget",
  "properties": {
    "title": "Erstaunliches Widget",           // From translation
    "description": "Das beste Widget",          // From translation
    "author": "user-456",                       // Not translatable, from base
    "created_at": "2025-01-01T00:00:00Z"       // Metadata, from base
  },
  "_locale": "de",
  "_fallback_chain": ["de", "en"]
}
```

### SQL Queries with LOCALE

RaisinSQL supports locale-aware queries using the `LOCALE` clause:

```sql
-- Query with specific locale
SELECT id, properties ->> 'title' AS title
FROM products
WHERE status = 'published'
LOCALE 'de';

-- Locale is applied to all property access automatically
SELECT *
FROM articles
WHERE properties ->> 'category' = 'Technology'
  AND properties ->> 'status' = 'published'
LOCALE 'fr';

-- Time-travel with locale
SELECT *
FROM products@revision_100
WHERE path LIKE '/catalog/%'
LOCALE 'de-CH';
```

**LOCALE Behavior:**
- Applies translations to all `properties ->>` operations
- Follows configured fallback chains
- Hidden nodes are filtered out for specified locale
- Default locale used if LOCALE clause omitted

### Historical Translation Queries

Query translations as they existed at any revision:

```
GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/{*path}?lang=de
```

Use cases:
- Compare translation changes over time
- Audit translation history
- Restore previous translations

---

## Translation Workflows

### Workflow 1: Progressive Translation

1. **Create base content** (default language)
   ```
   POST /api/repository/shop/main/head/products/
   { "name": "widget", "properties": { "title": "Amazing Widget" } }
   ```

2. **Add German translation**
   ```
   POST /api/repository/shop/main/head/products/widget/raisin:cmd/translate
   { "locale": "de", "translations": { "/title": "Erstaunliches Widget" } }
   ```

3. **Add French translation**
   ```
   POST /api/repository/shop/main/head/products/widget/raisin:cmd/translate
   { "locale": "fr", "translations": { "/title": "Widget Incroyable" } }
   ```

### Workflow 2: Localization Branch

1. **Create localization branch**
   ```
   POST /api/management/repositories/default/shop/branches
   { "name": "i18n/german", "from_revision": 100 }
   ```

2. **Add translations on branch**
   ```
   POST /api/repository/shop/i18n~german/head/products/widget/raisin:cmd/translate
   { "locale": "de", "translations": { "/title": "..." } }
   ```

3. **Review and merge to main**
   ```
   POST /api/management/repositories/default/shop/branches/main/merge
   { "from_branch": "i18n/german" }
   ```

### Workflow 3: Market-Specific Hiding

1. **Hide US promotion in EU markets**
   ```
   POST /api/repository/shop/main/head/promos/summer-sale/raisin:cmd/hide-in-locale
   { "locale": "de" }
   ```

2. **Query German market** - promotion automatically filtered
   ```
   GET /api/repository/shop/main/head/promos/?lang=de
   ```

3. **Unhide when EU version ready**
   ```
   POST /api/repository/shop/main/head/promos/summer-sale/raisin:cmd/delete-translation
   { "locale": "de", "message": "Unhide after localization" }
   ```

---

## Best Practices

### 1. Configure Fallback Chains Thoughtfully

```json
{
  "locale_fallback_chains": {
    // Regional → Language → Default
    "de-CH": ["de-CH", "de", "en"],  // Swiss German → German → English
    "en-GB": ["en-GB", "en"],         // British → American English
    "zh-TW": ["zh-TW", "zh", "en"]    // Traditional → Simplified → English
  }
}
```

### 2. Mark Properties as Translatable Appropriately

```yaml
# Translatable: user-visible text
- name: title
  translatable: true

# Not translatable: references, metadata, system fields
- name: author
  type: Reference
  translatable: false

- name: created_at
  type: Date
  translatable: false
```

### 3. Use Atomic Translation Updates

Group related property translations in a single API call to create one revision:

```json
{
  "locale": "de",
  "translations": {
    "/title": "Titel",
    "/description": "Beschreibung",
    "/keywords": ["Wort1", "Wort2"]
  }
}
```

### 4. Version Translation Releases

Tag complete translation milestones:

```bash
# Tag when German translation complete
POST /api/management/repositories/default/shop/tags
{ "name": "v1.0-de", "revision": 150 }
```

### 5. Hide Nodes During Translation

Hide nodes in target locales until translation is complete:

```bash
# Hide until ready
POST .../raisin:cmd/hide-in-locale { "locale": "de" }

# Translate
POST .../raisin:cmd/translate { "locale": "de", "translations": {...} }

# Unhide when done
POST .../raisin:cmd/delete-translation { "locale": "de" }
```

---

## Admin Console Support

The RaisinDB Admin Console provides a UI for managing translations:

### Language Switcher
- Select active locale from supported languages
- Persists selection across page navigation
- Shows content with fallback indicators

### Translation Editor
- Edit mode automatically switches to translation mode when locale ≠ default
- Shows original (default language) text as reference
- Highlights properties using fallback values
- Only shows translatable properties

### Translation Management
- View available translations per node
- Create/update/delete translations via UI
- Hide/unhide nodes in specific locales

---

## Next Steps

- **[NodeType Reference](../../model/nodetypes/overview.md)** — Learn how to mark properties as translatable
- **[RaisinSQL Guide](../sql/raisinsql.md)** — Use LOCALE in SQL queries
- **[REST API Overview](./overview.md)** — Explore all HTTP endpoints
- **[Concepts](../../why/concepts.md)** — Understand RaisinDB's data models
