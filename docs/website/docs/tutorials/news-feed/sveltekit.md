---
sidebar_label: "SvelteKit"
sidebar_position: 2
---

# SvelteKit Implementation

## Tech Stack

| Component | Technology | Version |
|-----------|------------|---------|
| **Framework** | SvelteKit | 2.x |
| **Language** | TypeScript | 5.x |
| **UI Library** | Svelte | 5.x |
| **Database Driver** | pg (node-postgres) | 8.x |
| **Styling** | TailwindCSS | 4.x |
| **Icons** | lucide-svelte | - |
| **Markdown** | marked | 17.x |

## RaisinDB Connection

```typescript
// src/lib/server/db.ts
import pg from 'pg';

const pool = new pg.Pool({
  connectionString: process.env.DATABASE_URL
  // PostgreSQL-compatible - standard pg library works!
});

export { pool };
```

## Application Structure

```
src/
├── lib/
│   ├── server/
│   │   └── db.ts              # PostgreSQL connection pool
│   ├── components/
│   │   ├── ArticleCard.svelte # Article preview card
│   │   ├── ArticleRow.svelte  # List row variant
│   │   ├── CategoryTabs.svelte
│   │   ├── TagBadge.svelte
│   │   ├── TagPicker.svelte   # Tag selection UI
│   │   ├── SearchInput.svelte
│   │   ├── ConnectionPicker.svelte  # Article relationships
│   │   ├── ConnectionModal.svelte
│   │   └── graph/
│   │       ├── SmartRelatedArticles.svelte
│   │       ├── BalancedViewWidget.svelte  # Contradicting views
│   │       └── EvidenceSourcesWidget.svelte
│   ├── stores/
│   │   └── toast.ts           # Notifications
│   └── utils.ts
└── routes/
    ├── +layout.svelte         # App shell with navigation
    ├── +page.svelte           # Home: featured + recent articles
    ├── search/
    │   ├── +page.svelte       # Search UI
    │   └── +page.server.ts    # Keyword/tag search queries
    ├── articles/[...path]/
    │   ├── +page.server.ts    # Article detail + graph data
    │   ├── edit/+page.svelte  # Edit article form
    │   └── move/+page.svelte  # Move to category
    ├── article/
    │   ├── new/+page.server.ts
    │   └── [id]/+page.server.ts
    ├── categories/+page.server.ts
    ├── settings/
    │   ├── +layout.svelte
    │   ├── tags/+page.svelte     # Tag management
    │   └── categories/+page.svelte
    └── api/
        └── pool-stats/+server.ts  # Connection pool monitoring
```

## Page Navigation Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Navigation Bar                                             │
│  [Home] [Tech] [Business] [Sports] [Entertainment] [Search] │
└─────────────────────────────────────────────────────────────┘
                              │
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
    │    Home     │    │  Category   │    │   Search    │
    │  Featured   │    │  Articles   │    │  Results    │
    │  + Recent   │    │  by Path    │    │  by Tag/    │
    └──────┬──────┘    └──────┬──────┘    │  Keyword    │
           │                  │           └──────┬──────┘
           └────────────┬─────┴──────────────────┘
                        ▼
              ┌─────────────────────┐
              │   Article Detail    │
              ├─────────────────────┤
              │ Title, Body, Tags   │
              │ ─────────────────── │
              │ Graph Widgets:      │
              │ • Timeline (series) │
              │ • Related Articles  │
              │ • Opposing Views    │
              │ • Evidence Sources  │
              │ • Shared Tags       │
              └─────────────────────┘
```

## Project Setup

```bash
# Clone the demo
git clone https://github.com/maravilla-labs/raisindb.git
cd raisindb/examples/demo/news-feed

# Install dependencies
npm install

# Configure database (create .env)
echo "DATABASE_URL=postgresql://default:password@localhost:5432/your_db" > .env

# Run development server
npm run dev
```

Open http://localhost:5173

## Key Code Patterns

### Server Load with RLS Context

```typescript
// src/routes/+page.server.ts
import type { PageServerLoad } from './$types';
import { pool } from '$lib/server/db';

export const load: PageServerLoad = async ({ cookies }) => {
  const token = cookies.get('access_token');
  const client = await pool.connect();

  try {
    // Set user context for Row-Level Security
    if (token) {
      await client.query('SET app.user = $1', [token]);
    }

    // Featured articles
    const featured = await client.query(`
      SELECT id, path, name, properties, created_at
      FROM social
      WHERE DESCENDANT_OF('/news/articles')
        AND node_type = 'news:Article'
        AND properties @> '{"featured": true, "status": "published"}'
      ORDER BY properties ->> 'publishing_date' DESC
      LIMIT 3
    `);

    // Recent articles
    const recent = await client.query(`
      SELECT id, path, name, properties, created_at
      FROM social
      WHERE DESCENDANT_OF('/news/articles')
        AND node_type = 'news:Article'
        AND properties ->> 'status' = 'published'
      ORDER BY properties ->> 'publishing_date' DESC
      LIMIT 10
    `);

    return {
      featured: featured.rows,
      recent: recent.rows
    };
  } finally {
    if (token) {
      await client.query('RESET app.user');
    }
    client.release();
  }
};
```

### Dynamic Navigation from Database

```typescript
// src/routes/+layout.server.ts
export const load: PageServerLoad = async () => {
  const client = await pool.connect();
  try {
    // Categories come from the database hierarchy
    const categories = await client.query(`
      SELECT path, name, properties
      FROM social
      WHERE CHILD_OF('/news/articles')
        AND node_type = 'news:Category'
      ORDER BY properties ->> 'sort_order' ASC
    `);

    return { categories: categories.rows };
  } finally {
    client.release();
  }
};
```

### Graph Relationship Component

```svelte
<!-- src/lib/components/graph/SmartRelatedArticles.svelte -->
<script lang="ts">
  export let articles: Array<{
    path: string;
    title: string;
    relationType: string;
    relevance: number;
  }>;
</script>

<div class="space-y-2">
  <h3 class="font-semibold">Related Articles</h3>
  {#each articles as article}
    <a href="/articles{article.path}" class="block p-2 hover:bg-gray-100 rounded">
      <span class="text-sm text-gray-500">{article.relationType}</span>
      <p>{article.title}</p>
      <div class="w-full bg-gray-200 h-1 rounded">
        <div class="bg-blue-500 h-1 rounded" style="width: {article.relevance}%"></div>
      </div>
    </a>
  {/each}
</div>
```

---

## Source Code

Full implementation: [news-feed](https://github.com/maravilla-labs/raisindb/tree/main/examples/demo/news-feed)
