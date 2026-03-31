# News Feed PHP Demo

A Laravel 12 demo application showcasing RaisinDB integration via the PostgreSQL wire protocol. This app demonstrates how to build a news/blog platform using RaisinDB's hierarchical data model, graph queries, and property-based filtering.

## Tech Stack

- **PHP 8.2+**
- **Laravel 12** - Web framework
- **Tailwind CSS 4** - Styling (via Vite)
- **Blade Lucide Icons** - Icon library
- **Graham Campbell Markdown** - Markdown rendering
- **RaisinDB** - Database (via PostgreSQL wire protocol)

## Prerequisites

- PHP 8.2 or higher
- Composer
- Node.js 18+ and npm
- RaisinDB server running with pgwire enabled (default port: 5433)

## Quick Start

```bash
# Install dependencies
composer install
npm install

# Copy environment file
cp .env.example .env

# Generate application key
php artisan key:generate

# Configure RaisinDB connection (see below)
# Edit .env with your RaisinDB settings

# Build frontend assets
npm run build

# Start development server
php artisan serve
```

Or use the composer script:

```bash
composer run dev
```

This starts the Laravel server, queue worker, log viewer, and Vite in parallel.

## RaisinDB Configuration

Add the following to your `.env` file:

```env
DB_CONNECTION=pgsql
DB_HOST=127.0.0.1
DB_PORT=5433
DB_DATABASE=social
DB_USERNAME=superbigshit
DB_PASSWORD=superbigshit
```

The app uses the `social` workspace and expects content under `/superbigshit/articles/` and `/superbigshit/tags/`.

## Project Structure

```
app/
├── Http/Controllers/
│   ├── ArticleController.php      # Article CRUD and display
│   ├── SearchController.php       # Full-text search
│   └── Settings/
│       ├── CategorySettingsController.php
│       └── TagController.php
│
├── Services/RaisinDB/             # RaisinDB integration layer
│   ├── RaisinQueryBuilder.php     # Fluent query builder for RaisinDB
│   ├── GraphQueryBuilder.php      # GRAPH_TABLE and NEIGHBORS queries
│   ├── ArticleService.php         # Article-specific operations
│   ├── CategoryService.php        # Category management
│   └── TagService.php             # Tag management
│
resources/views/
├── layouts/app.blade.php          # Main layout
├── home.blade.php                 # Homepage
├── articles/
│   ├── show.blade.php             # Article detail with graph data
│   ├── create.blade.php           # New article form
│   └── edit.blade.php             # Edit article form
├── components/
│   ├── article-card.blade.php     # Article card component
│   ├── tag-chip.blade.php         # Tag display component
│   └── graph/                     # Graph visualization components
│       ├── correction-banner.blade.php
│       ├── story-timeline.blade.php
│       ├── balanced-view.blade.php
│       └── smart-related.blade.php
```

## RaisinDB Integration Patterns

### Query Builder (`RaisinQueryBuilder`)

Fluent interface for building RaisinDB SQL queries:

```php
use App\Services\RaisinDB\RaisinQueryBuilder;

// Find published articles in a category
$articles = RaisinQueryBuilder::query('social')
    ->descendantOf('/superbigshit/articles')
    ->whereNodeType('news:Article')
    ->wherePropertyEquals('status', 'published')
    ->orderByProperty('publishing_date', 'DESC')
    ->limit(10)
    ->get();

// Find by exact path
$article = RaisinQueryBuilder::query('social')
    ->wherePath('/superbigshit/articles/tech/my-article')
    ->first();

// JSONB containment queries
$featured = RaisinQueryBuilder::query('social')
    ->wherePropertiesContain(['featured' => true, 'status' => 'published'])
    ->get();
```

### Graph Queries (`GraphQueryBuilder`)

SQL/PGQ (ISO SQL:2023) pattern matching for relationships:

```php
use App\Services\RaisinDB\GraphQueryBuilder;

// Find related articles via similarity
$related = (new GraphQueryBuilder())
    ->match('(this:Article)-[r:`similar-to`]->(related:Article)')
    ->where("this.path = '/superbigshit/articles/tech/rust-guide'")
    ->columns([
        'related.id AS id',
        'related.path AS path',
        'r.weight AS score'
    ])
    ->orderBy('score', 'DESC')
    ->limit(5)
    ->get();

// Use NEIGHBORS() for simple traversals
$neighbors = GraphQueryBuilder::neighbors(
    'social:/superbigshit/articles/tech/rust-guide',
    'OUT',
    'tagged-with'
);

// Create relationships
GraphQueryBuilder::relate(
    '/superbigshit/articles/post1',
    '/superbigshit/articles/post2',
    'similar-to',
    0.85
);
```

### Static Helper Methods

Common operations are available as static methods:

```php
// Insert a new node
RaisinQueryBuilder::insert('social', [
    'path' => '/superbigshit/articles/tech/new-post',
    'node_type' => 'news:Article',
    'name' => 'new-post',
    'properties' => json_encode([
        'title' => 'My New Post',
        'status' => 'draft'
    ])
]);

// Update properties
RaisinQueryBuilder::updateProperty(
    'social',
    '/superbigshit/articles/tech/new-post',
    'status',
    'published'
);

// Increment a counter
RaisinQueryBuilder::incrementProperty(
    'social',
    '/superbigshit/articles/tech/new-post',
    'views',
    1
);
```

## RaisinDB-Specific SQL Features

### Hierarchical Predicates

```sql
-- All descendants (any depth)
SELECT * FROM social WHERE DESCENDANT_OF('/superbigshit/articles')

-- Direct children only
SELECT * FROM social WHERE CHILD_OF('/superbigshit/articles')
```

### Reference Index

```sql
-- Find nodes referencing a target
SELECT * FROM social WHERE REFERENCES('social:/superbigshit/tags/rust')
```

### JSONB Property Access

```sql
-- Extract text value
WHERE properties ->> 'status'::TEXT = 'published'

-- JSONB containment
WHERE properties @> '{"featured": true}'::JSONB

-- Cast for comparisons
WHERE (properties ->> 'views')::int > 100
```

### Graph Queries (SQL/PGQ)

```sql
SELECT * FROM GRAPH_TABLE(
    MATCH (a:Article)-[r:`similar-to`]->(b:Article)
    WHERE a.path = '/superbigshit/articles/tech/rust-guide'
    COLUMNS (b.id, b.path, r.weight AS score)
) AS g
ORDER BY g.score DESC
```

## Key Differences from Standard PostgreSQL

1. **Workspace as Table**: Query `FROM social` where `social` is the workspace name
2. **Hierarchical Functions**: `DESCENDANT_OF()`, `CHILD_OF()` without path parameter syntax
3. **Reference Queries**: `REFERENCES('workspace:/path')` for finding referencing nodes
4. **Graph Patterns**: Full SQL/PGQ support with `GRAPH_TABLE` and `NEIGHBORS()`
5. **JSONB Properties**: All node properties stored in `properties` JSONB column
6. **Parameter Casting**: Bound parameters may need explicit type casts (e.g., `?::int`)

## Development

```bash
# Run development server with hot reload
composer run dev

# Or manually:
php artisan serve        # Laravel server
npm run dev              # Vite dev server

# Run tests
composer run test

# Format code
./vendor/bin/pint
```

## Routes

| Method | URI | Description |
|--------|-----|-------------|
| GET | `/` | Homepage with featured/recent articles |
| GET | `/search` | Full-text search |
| GET | `/articles/{path}` | View article or category |
| GET | `/articles/new` | Create article form |
| POST | `/articles/new` | Store new article |
| GET | `/articles/{path}/edit` | Edit article form |
| PUT | `/articles/{path}` | Update article |
| DELETE | `/articles/{path}` | Delete article |
| GET | `/settings/categories` | Manage categories |
| GET | `/settings/tags` | Manage tags |

## License

MIT
