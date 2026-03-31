# Launchpad - RaisinDB Demo Application

A minimal demo showing how to build a content-driven application with RaisinDB.

## Slash Commands

| Command | Description |
|---------|-------------|
| `/package-validate` | Validate package structure |
| `/package-build` | Create .rap file from package folder |
| `/package-upload` | Upload latest .rap to RaisinDB server |
| `/package-deploy` | Full deploy (validate + build + upload) |
| `/frontend-install` | Install frontend dependencies |
| `/frontend-dev` | Start frontend dev server |

## Deployment Workflow

```bash
# 1. Validate and build package
/package-deploy

# 2. Install package on server (manual step in admin console)

# 3. Start frontend
/frontend-dev
```

---

## Package Structure

A RaisinDB package is a folder with content definitions that gets bundled into a `.rap` file.

```
package/
├── manifest.yaml          # Package metadata and what it provides
├── nodetypes/             # Define data structures (schema)
│   └── page.yaml
├── archetypes/            # Page templates (what fields a page type has)
│   ├── landing-page.yaml
│   └── kanban-board.yaml
├── elementtypes/          # Reusable content blocks
│   ├── hero.yaml
│   ├── text-block.yaml
│   └── feature-grid.yaml
├── workspaces/            # Workspace definitions
│   └── launchpad.yaml
├── content/               # Initial content and functions
│   ├── launchpad/         # Content for the launchpad workspace
│   └── functions/         # Server-side functions and triggers
└── static/                # Static assets (images, etc.)
```

### manifest.yaml

Defines what the package provides:

```yaml
name: launchpad
version: 1.0.7
title: Launchpad
description: Demo application

provides:
  nodetypes:
    - launchpad:Page
  archetypes:
    - launchpad:LandingPage
    - launchpad:KanbanBoard
  elementtypes:
    - launchpad:Hero
    - launchpad:TextBlock
  workspaces:
    - launchpad
  functions:
    - /functions/lib/launchpad/handle-friendship-request
  triggers:
    - /functions/triggers/on-friendship-request

workspace_patches:
  launchpad:
    allowed_node_types:
      add:
        - launchpad:Page
        - raisin:Folder
```

---

## Content Model Concepts

### 1. NodeType (Schema)

Defines the **data structure** for a type of content. Like a database table schema.

**Example:** `nodetypes/page.yaml`
```yaml
name: launchpad:Page
title: Page
description: Base page type
icon: file-text

properties:
  - name: title
    type: String
    required: true
    index: [Fulltext]
  - name: slug
    type: String
    required: true
  - name: description
    type: String
    required: false

versionable: true
publishable: true
indexable: true
```

**Property Types:** `String`, `Number`, `Boolean`, `Date`, `Object`, `Array`

### 2. Archetype (Page Template)

Defines the **editor experience** for a page type. Specifies what fields appear in the admin UI.

**Example:** `archetypes/landing-page.yaml`
```yaml
name: launchpad:LandingPage
title: Landing Page
base_node_type: launchpad:Page    # Links to the NodeType

fields:
  - $type: TextField
    name: title
    title: Page Title
    required: true

  - $type: TextField
    name: slug
    title: URL Slug

  - $type: SectionField              # Container for elements
    name: content
    title: Page Content
    allowed_element_types:           # What blocks can be added
      - launchpad:Hero
      - launchpad:TextBlock
      - launchpad:FeatureGrid
```

**Field Types:** `TextField`, `TextareaField`, `NumberField`, `BooleanField`, `DateField`, `MediaField`, `SectionField`, `CompositeField`

### 3. ElementType (Content Block)

Defines a **reusable content block** that can be placed in SectionFields.

**Example:** `elementtypes/hero.yaml`
```yaml
name: launchpad:Hero
title: Hero Section
icon: image

fields:
  - $type: TextField
    name: headline
    title: Headline
    required: true

  - $type: TextField
    name: subheadline
    title: Subheadline

  - $type: TextField
    name: cta_text
    title: CTA Button Text

  - $type: TextField
    name: cta_link
    title: CTA Button Link

  - $type: MediaField
    name: background_image
    title: Background Image
```

### Data Flow

```
NodeType (schema) ← Archetype (template) ← ElementTypes (blocks)
     ↓                    ↓                      ↓
 Database            Admin Editor           Inline Blocks
```

---

## Frontend Integration

### Project Structure

```
frontend/
├── src/
│   ├── lib/
│   │   ├── raisin.ts              # RaisinDB client setup
│   │   ├── stores/
│   │   │   ├── auth.ts            # Authentication store
│   │   │   ├── connection.ts      # WebSocket connection
│   │   │   └── messaging.ts       # Messaging utilities
│   │   └── components/
│   │       ├── pages/             # Archetype → Component
│   │       │   ├── index.ts       # Page component registry
│   │       │   └── LandingPage.svelte
│   │       └── elements/          # ElementType → Component
│   │           ├── index.ts       # Element component registry
│   │           └── Hero.svelte
│   └── routes/
│       ├── +layout.svelte         # App layout
│       ├── +layout.ts             # Root data loading
│       └── [...slug]/             # Dynamic page routing
│           ├── +page.ts           # Load page data
│           └── +page.svelte       # Render page
```

### RaisinDB Client Setup (`lib/raisin.ts`)

```typescript
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

const RAISIN_URL = 'ws://localhost:8081/sys/default/launchpad';
const REPOSITORY = 'launchpad';
const WORKSPACE = 'launchpad';

let clientInstance: RaisinClient | null = null;
let dbInstance: Database | null = null;

export function getClient(): RaisinClient {
  if (!clientInstance) {
    clientInstance = new RaisinClient(RAISIN_URL, {
      tokenStorage: new LocalStorageTokenStorage('launchpad'),
      tenantId: 'default',
      defaultBranch: 'main',
    });
  }
  return clientInstance;
}

// Initialize session - call once on app start
export async function initSession(): Promise<IdentityUser | null> {
  const client = getClient();
  return client.initSession(REPOSITORY);
}

// Get database instance for SQL queries
export async function getDatabase(): Promise<Database> {
  if (!dbInstance) {
    const client = getClient();
    dbInstance = client.database(REPOSITORY);
  }
  return dbInstance;
}
```

### SQL Queries

```typescript
// Execute SQL query
export async function query<T>(sql: string, params?: unknown[]): Promise<T[]> {
  const db = await getDatabase();
  const result = await db.executeSql(sql, params);
  return (result.rows ?? []) as T[];
}

// Get single result
export async function queryOne<T>(sql: string, params?: unknown[]): Promise<T | null> {
  const rows = await query<T>(sql, params);
  return rows[0] ?? null;
}
```

**Common Query Patterns:**

```typescript
// Fetch page by path
const page = await queryOne(`
  SELECT id, path, name, node_type, archetype, properties
  FROM launchpad
  WHERE path = $1
`, ['/launchpad/home']);

// Get children of a folder
const pages = await query(`
  SELECT id, path, name, properties
  FROM launchpad
  WHERE CHILD_OF('/launchpad')
    AND node_type = 'launchpad:Page'
`);

// Query with property filter (JSONB)
const users = await query(`
  SELECT id, path, properties
  FROM "raisin:access_control"
  WHERE node_type = 'raisin:User'
    AND properties->>'email' = $1
`, ['user@example.com']);

// Insert a new node (path column required, name extracted from path)
await query(`
  INSERT INTO "raisin:access_control" (path, node_type, properties)
  VALUES ($1, 'raisin:Message', $2::jsonb)
`, ['/users/abc/outbox/msg-123', JSON.stringify({ message_type: 'test' })]);

// Update properties
await query(`
  UPDATE "raisin:access_control"
  SET properties = $1::jsonb
  WHERE path = $2
`, [JSON.stringify({ data: { name: 'John' } }), '/users/abc/profile']);
```

### Page Component Registry (`components/pages/index.ts`)

Maps archetype names to Svelte components:

```typescript
import LandingPage from './LandingPage.svelte';
import KanbanBoardPage from './KanbanBoardPage.svelte';

export const pageComponents: Record<string, Component<any>> = {
  'launchpad:LandingPage': LandingPage,
  'launchpad:KanbanBoard': KanbanBoardPage,
};
```

### Element Component Registry (`components/elements/index.ts`)

Maps element type names to Svelte components:

```typescript
import Hero from './Hero.svelte';
import TextBlock from './TextBlock.svelte';

export const elementComponents: Record<string, Component<any>> = {
  'launchpad:Hero': Hero,
  'launchpad:TextBlock': TextBlock,
};
```

### Page Component Example (`LandingPage.svelte`)

Renders a page by iterating over its `content` elements:

```svelte
<script lang="ts">
  import { elementComponents } from '$lib/components/elements';
  import type { PageNode, Element } from '$lib/raisin';

  let { page }: { page: PageNode } = $props();

  const elements: Element[] = $derived(page.properties.content ?? []);
</script>

<article>
  {#each elements as element (element.uuid)}
    {@const Component = elementComponents[element.element_type]}
    {#if Component}
      <Component {element} />
    {:else}
      <div>Unknown: {element.element_type}</div>
    {/if}
  {/each}
</article>
```

### Element Component Example (`Hero.svelte`)

Renders a single content block:

```svelte
<script lang="ts">
  interface HeroElement {
    uuid: string;
    element_type: string;
    headline?: string;
    subheadline?: string;
    cta_text?: string;
    cta_link?: string;
  }

  let { element }: { element: HeroElement } = $props();
</script>

<section class="hero">
  {#if element.headline}
    <h1>{element.headline}</h1>
  {/if}
  {#if element.subheadline}
    <p>{element.subheadline}</p>
  {/if}
  {#if element.cta_text && element.cta_link}
    <a href={element.cta_link}>{element.cta_text}</a>
  {/if}
</section>
```

### Dynamic Page Routing (`[...slug]/+page.ts`)

Load page data based on URL:

```typescript
import type { PageLoad } from './$types';
import { getPageByPath } from '$lib/raisin';

export const load: PageLoad = async ({ params }) => {
  const slug = params.slug || 'home';
  const page = await getPageByPath(slug);

  return { page };
};
```

---

## Authentication

### Setup

```typescript
// lib/stores/auth.ts
import { writable, derived } from 'svelte/store';
import { initSession, login, logout, onAuthStateChange } from '$lib/raisin';

function createAuthStore() {
  const { subscribe, set } = writable({ user: null, loading: true });

  return {
    subscribe,
    async init() {
      const user = await initSession();
      set({ user, loading: false });
    },
    async login(email: string, password: string) {
      return login(email, password);
    },
    async logout() {
      await logout();
    }
  };
}

export const auth = createAuthStore();
export const user = derived(auth, $auth => $auth.user);
```

### User Home Path

Authenticated users have a `home` path in the `raisin:access_control` workspace:

```typescript
const user = $user;
// user.home = '/raisin:access_control/users/abc123'
// user.email = 'user@example.com'

// Query user's inbox
const inboxPath = user.home.replace('/raisin:access_control', '') + '/inbox';
const messages = await query(`
  SELECT * FROM "raisin:access_control"
  WHERE CHILD_OF('/raisin:access_control${inboxPath}')
`);
```

---

## Functions & Triggers

### Creating a Function

Functions are JavaScript handlers that run on the server.

**File:** `package/content/functions/lib/launchpad/my-function/.node.yaml`
```yaml
node_type: "raisin:Function"
properties:
  name: "my-function"
  title: "My Function"
  description: "Does something useful"
  execution_mode: "async"
  enabled: true
  language: "javascript"
  entry_file: "index.js:handler"
```

**File:** `package/content/functions/lib/launchpad/my-function/index.js`
```javascript
async function handler(context) {
  const { event, workspace } = context.flow_input;

  // Query nodes
  const result = await raisin.sql.query(`
    SELECT * FROM "raisin:access_control"
    WHERE properties->>'email' = $1
  `, ['user@example.com']);

  // Create nodes
  await raisin.nodes.create('raisin:access_control', '/users/abc/inbox', {
    name: 'message-123',
    node_type: 'raisin:Message',
    properties: { subject: 'Hello' }
  });

  return { success: true };
}
```

### Creating a Trigger

Triggers fire functions in response to node events.

**File:** `package/content/functions/triggers/on-something/.node.yaml`
```yaml
node_type: raisin:Trigger
properties:
  title: On Something
  name: launchpad-on-something
  description: Fires when something happens
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Created
      - Updated
  filters:
    workspaces:
      - "raisin:access_control"
    paths:
      - "users/*/outbox/*"
    node_types:
      - raisin:Message
    property_filters:
      message_type: "friendship_request"
  priority: 10
  max_retries: 3
  function_path: /functions/lib/launchpad/my-function
```

---

## Key Files Reference

| File | Purpose |
|------|---------|
| `package/manifest.yaml` | Package definition |
| `package/nodetypes/*.yaml` | Data schemas |
| `package/archetypes/*.yaml` | Page templates |
| `package/elementtypes/*.yaml` | Content blocks |
| `frontend/src/lib/raisin.ts` | Client setup & queries |
| `frontend/src/lib/stores/auth.ts` | Auth state |
| `frontend/src/lib/components/pages/index.ts` | Archetype→Component map |
| `frontend/src/lib/components/elements/index.ts` | Element→Component map |
| `frontend/src/routes/[...slug]/+page.ts` | Dynamic page loading |

---

## CLI Commands

```bash
# Create package
raisindb package create ./package

# Upload to server
raisindb package upload launchpad-1.0.7.rap -r launchpad

# Interactive shell
raisindb shell
```

## Tech Stack

- **Backend:** RaisinDB (WebSocket server on `ws://localhost:8081`)
- **Frontend:** SvelteKit 2.x + Svelte 5 (runes)
- **Styling:** TailwindCSS 4.x
- **Icons:** Lucide Svelte
- **SDK:** `@raisindb/client` (local package)
