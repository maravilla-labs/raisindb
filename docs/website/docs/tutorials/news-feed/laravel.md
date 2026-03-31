---
sidebar_label: "Laravel"
sidebar_position: 4
---

# Laravel Implementation

## Tech Stack

| Component | Technology | Version |
|-----------|------------|---------|
| **Framework** | Laravel | 12.x |
| **Language** | PHP | 8.2+ |
| **Database Access** | Eloquent / DB Facade (PDO) | - |
| **Database Driver** | pgsql (PDO PostgreSQL) | - |
| **Templates** | Blade | - |
| **Styling** | TailwindCSS (Vite) | - |
| **Icons** | blade-lucide-icons | - |
| **Markdown** | graham-campbell/markdown | 16.x |
| **Build** | Vite | - |

## RaisinDB Connection

```php
// config/database.php
'pgsql' => [
    'driver' => 'pgsql',
    'host' => env('DB_HOST', 'localhost'),
    'port' => env('DB_PORT', '5432'),
    'database' => env('DB_DATABASE', 'your_database'),
    'username' => env('DB_USERNAME', 'default'),
    'password' => env('DB_PASSWORD', ''),
    'charset' => 'utf8',
    'prefix' => '',
    'schema' => 'public',
],
```

```env
# .env
DB_CONNECTION=pgsql
DB_HOST=localhost
DB_PORT=5432
DB_DATABASE=your_database
DB_USERNAME=default
DB_PASSWORD=your_password
```

## Application Structure

```
app/
├── Http/
│   ├── Controllers/
│   │   ├── HomeController.php        # / - home page
│   │   ├── ArticleController.php     # /articles/** - CRUD
│   │   ├── CategoryController.php
│   │   ├── SearchController.php      # /search
│   │   └── SettingsController.php    # /settings/**
│   └── Middleware/
│       └── RaisinDbUserContext.php   # SET app.user middleware
├── Models/
│   └── User.php
├── Services/
│   └── RaisinDbService.php           # Query helpers
└── Providers/

resources/views/
├── layouts/
│   └── app.blade.php                 # Main layout
├── home.blade.php                    # Home page
├── articles/
│   ├── index.blade.php               # Article list
│   ├── show.blade.php                # Article detail + graph
│   ├── create.blade.php
│   └── edit.blade.php
├── auth/
│   ├── login.blade.php
│   └── register.blade.php
├── categories/
│   └── show.blade.php
├── search/
│   └── index.blade.php
├── settings/
│   ├── tags.blade.php
│   └── categories.blade.php
└── components/
    ├── article-card.blade.php
    ├── tag-badge.blade.php
    ├── category-tabs.blade.php
    ├── graph-timeline.blade.php      # Article series
    ├── graph-related.blade.php       # Similar articles
    └── graph-opposing.blade.php      # Contradicting views

routes/
├── web.php                           # Web routes
└── api.php                           # API routes

database/
├── migrations/
└── seeders/
```

## Page Navigation Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Blade Layout (layouts/app.blade.php)                       │
│  [Home] [Tech] [Business] [Sports] [Entertainment] [Search] │
└─────────────────────────────────────────────────────────────┘
                              │
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
    HomeController     ArticleController   SearchController
    Route::get('/')    Route::get('/articles/{cat}')
           │                  │
           └────────────┬─────┴───────────────────┘
                        ▼
              ArticleController@show
              Route::get('/articles/{category}/{slug}')
                        │
                        ▼
              ┌─────────────────────────┐
              │  Blade Components:      │
              │  • article-card         │
              │  • graph-timeline       │
              │  • graph-related        │
              │  • graph-opposing       │
              └─────────────────────────┘
```

## Project Setup

```bash
# Clone the demo
git clone https://github.com/maravilla-labs/raisindb.git
cd raisindb/examples/demo/news-feed-php

# Install PHP dependencies
composer install

# Install JS dependencies (TailwindCSS)
npm install

# Configure environment
cp .env.example .env
php artisan key:generate

# Edit .env with your RaisinDB credentials
# DB_CONNECTION=pgsql
# DB_HOST=localhost
# DB_DATABASE=your_database
# DB_USERNAME=default
# DB_PASSWORD=your_password

# Run development server
composer dev
# Or separately:
# php artisan serve
# npm run dev
```

Open http://localhost:8000

## Key Code Patterns

### Service Class for RaisinDB Queries

```php
<?php
// app/Services/RaisinDbService.php

namespace App\Services;

use Illuminate\Support\Facades\DB;

class RaisinDbService
{
    protected ?string $userToken = null;

    public function setUserContext(?string $token): self
    {
        $this->userToken = $token;
        return $this;
    }

    public function query(string $sql, array $bindings = []): array
    {
        if ($this->userToken) {
            DB::statement('SET app.user = ?', [$this->userToken]);
        }

        try {
            $results = DB::select($sql, $bindings);
            return array_map(fn($row) => (array) $row, $results);
        } finally {
            if ($this->userToken) {
                DB::statement('RESET app.user');
            }
        }
    }

    public function findFeaturedArticles(int $limit = 5): array
    {
        return $this->query("
            SELECT id, path, name, properties, created_at
            FROM social
            WHERE DESCENDANT_OF('/news/articles')
              AND node_type = 'news:Article'
              AND properties @> '{\"featured\": true, \"status\": \"published\"}'
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
        ", [$limit]);
    }

    public function findByTag(string $tagPath, int $limit = 10): array
    {
        $reference = "social:{$tagPath}";
        return $this->query("
            SELECT id, path, name, properties
            FROM social
            WHERE REFERENCES(?)
              AND node_type = 'news:Article'
              AND properties ->> 'status' = 'published'
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT ?
        ", [$reference, $limit]);
    }
}
```

### Dynamic Navigation from Database

```php
<?php
// app/Http/Controllers/Controller.php or ViewComposer

public function getCategories(): array
{
    return DB::select("
        SELECT path, name, properties
        FROM social
        WHERE CHILD_OF('/news/articles')
          AND node_type = 'news:Category'
        ORDER BY properties ->> 'sort_order' ASC
    ");
}
```

### Controller Using the Service

```php
<?php
// app/Http/Controllers/HomeController.php

namespace App\Http\Controllers;

use App\Services\RaisinDbService;
use Illuminate\Http\Request;

class HomeController extends Controller
{
    public function __construct(
        protected RaisinDbService $raisinDb
    ) {}

    public function index(Request $request)
    {
        // Set RLS context from session/cookie
        $token = $request->cookie('access_token');
        $this->raisinDb->setUserContext($token);

        // Fetch articles
        $featured = $this->raisinDb->findFeaturedArticles(3);
        $recent = $this->raisinDb->query("
            SELECT id, path, name, properties, created_at
            FROM social
            WHERE DESCENDANT_OF('/news/articles')
              AND node_type = 'news:Article'
              AND properties ->> 'status' = 'published'
            ORDER BY properties ->> 'publishing_date' DESC
            LIMIT 10
        ");

        // Decode JSONB properties
        $featured = $this->decodeProperties($featured);
        $recent = $this->decodeProperties($recent);

        return view('home', compact('featured', 'recent'));
    }

    protected function decodeProperties(array $rows): array
    {
        return array_map(function ($row) {
            $row['properties'] = json_decode($row['properties'], true);
            return $row;
        }, $rows);
    }
}
```

### Graph Queries in Laravel

```php
<?php
// app/Services/RaisinDbService.php (additional methods)

public function findArticleSeries(string $articlePath): array
{
    $safePath = addslashes($articlePath);
    return $this->query("
        SELECT * FROM GRAPH_TABLE(
            MATCH (this)-[:continues*]->(prev)
            WHERE this.path = '{$safePath}'
            COLUMNS (
                prev.id AS id,
                prev.path AS path,
                prev.name AS name,
                prev.properties AS properties
            )
        ) AS g
        ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC
    ");
}

public function findRelatedArticles(string $articlePath, int $limit = 5): array
{
    $safePath = addslashes($articlePath);
    return $this->query("
        SELECT * FROM GRAPH_TABLE(
            MATCH (this)-[r:similar-to|see-also]->(related)
            WHERE this.path = '{$safePath}'
            COLUMNS (
                related.path AS path,
                related.name AS title,
                related.properties AS properties,
                r.type AS relation_type,
                r.weight AS relevance
            )
        ) AS g
        ORDER BY g.relevance DESC
        LIMIT {$limit}
    ");
}

public function createRelation(string $from, string $to, string $type, float $weight = 1.0): void
{
    $safeFrom = addslashes($from);
    $safeTo = addslashes($to);
    $safeType = addslashes($type);

    DB::statement("
        RELATE FROM path='{$safeFrom}' IN WORKSPACE 'social'
          TO path='{$safeTo}' IN WORKSPACE 'social'
          TYPE '{$safeType}' WEIGHT {$weight}
    ");
}
```

### Blade Component for Graph Widgets

```blade
{{-- resources/views/components/graph-related.blade.php --}}
@props(['articles'])

<div class="bg-white rounded-lg shadow p-4">
    <h3 class="font-semibold text-lg mb-3">Related Articles</h3>

    @forelse($articles as $article)
        <a href="/articles{{ $article['path'] }}"
           class="block p-2 hover:bg-gray-50 rounded mb-2">
            <span class="text-xs text-blue-600 uppercase">
                {{ $article['relation_type'] }}
            </span>
            <p class="font-medium">{{ $article['title'] }}</p>
            <div class="w-full bg-gray-200 h-1 rounded mt-1">
                <div class="bg-blue-500 h-1 rounded"
                     style="width: {{ $article['relevance'] * 100 }}%"></div>
            </div>
        </a>
    @empty
        <p class="text-gray-500 text-sm">No related articles found.</p>
    @endforelse
</div>
```

---

## Source Code

Full implementation: [news-feed-php](https://github.com/maravilla-labs/raisindb/tree/main/examples/demo/news-feed-php)
