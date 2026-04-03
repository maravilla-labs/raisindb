---
name: raisindb-overview
description: "Core concepts of RaisinDB content-driven applications. Use when building any RaisinDB app. Teaches: path-as-URL routing, archetype-to-component mapping, content modeling, project structure."
---

# RaisinDB Overview

## Critical Rules

1. **Always ask the user for the repository name** before writing any frontend code. The repository is a server-side concept (like a database name) — it is NOT the same as the workspace or package name. The WebSocket URL uses the repository name: `ws://localhost:8080/sys/default/{REPOSITORY}`. Use it in `client.initSession(REPOSITORY)`, `client.database(REPOSITORY)`, and `client.loginWithEmail(email, password, REPOSITORY)`.

2. **Always validate after changing any YAML in `package/`**. Run this command every time you create or modify a `.yaml` or `.node.yaml` file:

       raisindb package create ./package --check

   Do NOT skip this step. Fix any errors before proceeding.

3. **For navigation queries, use a `hide_in_nav` Boolean property** on the NodeType instead of filtering by archetype or hardcoding path exclusions. Add `hide_in_nav: Boolean` to your Page NodeType and filter with `properties->>'hide_in_nav'::Boolean != true`.

4. **Boolean property queries**: Cast the key to `Boolean`, not `String`: `properties->>'featured'::Boolean = true`. For string properties, cast to `String`: `properties->>'status'::String = 'published'`.

## What RaisinDB Is

RaisinDB is a multi-tenant content database with SQL queries, graph traversal (Cypher/PGQ), real-time WebSocket subscriptions, and CRDT-based replication. You model content as typed nodes living at hierarchical paths, then build frontends that map those nodes to UI components. Every node belongs to a tenant, repository, branch, and workspace.

**Key distinction**: A **repository** is the server-side database you connect to (ask the user for this name). A **workspace** is a logical content partition within a repository (defined in your package YAML). A single repository can have multiple workspaces — use them to separate concerns (e.g., `content` for pages, `media` for shared files, `raisin:access_control` for users/roles).

## The Content-to-Component Pipeline

This is the core mental model. Understand this and you understand RaisinDB apps.

```
NodeType (schema)  -->  Archetype (page template)  -->  ElementTypes (content blocks)
      |                        |                              |
YAML in package/        Maps to Page Component        Maps to Element Components
nodetypes/              in pages/index.ts             in elements/index.ts
```

- **NodeType** defines the data schema (properties, types, indexes). Like a database table.
- **Archetype** defines a page template that references a NodeType. Each archetype maps to one frontend page component.
- **ElementType** defines a content block (hero, text, feature grid). Each element type maps to one frontend element component.

A page node has an `archetype` field and a `properties.content[]` array. Each entry in `content[]` has an `element_type` field. The frontend resolves both through registries.

## Path-as-URL Routing

Content nodes live at paths like `/workspace/home`, `/workspace/about/team`. The frontend uses a catch-all route (`/[...slug]`) that:

1. Takes the URL slug (e.g., `about/team`)
2. Queries: `SELECT * FROM workspace WHERE path = '/workspace/{slug}'`
3. Gets back a node with an `archetype` field (e.g., `myapp:LandingPage`)
4. Looks up the page component from the registry: `pageComponents[node.archetype]`
5. Renders the page component, passing the full node as a prop
6. The page component iterates `properties.content[]`, looks up each `element_type` in the element registry, renders each element component

```typescript
// +page.ts — load data
const slug = params.slug || 'home';
const page = await getPageByPath(`/${slug}`);
return { page };

// +page.svelte — resolve component
const PageComponent = pageComponents[data.page.archetype];
// <PageComponent page={data.page} />

// LandingPage.svelte — render elements
{#each page.properties.content as element}
  {@const Component = elementComponents[element.element_type]}
  <Component {element} />
{/each}
```

## Project Structure

```
my-app/
├── package/                    # RaisinDB content package (YAML)
│   ├── manifest.yaml           # Package name, version, provides list
│   ├── nodetypes/              # Data schemas (properties, types, indexes)
│   ├── archetypes/             # Page templates (fields, allowed elements)
│   ├── elementtypes/           # Content block definitions (fields)
│   ├── workspaces/             # Workspace configurations
│   └── content/                # Initial content, functions, triggers
│       ├── {workspace}/        # Seed content nodes
│       └── functions/          # Server-side JS functions + triggers
└── frontend/                   # SvelteKit or React app
    └── src/
        ├── routes/
        │   └── [...slug]/      # Catch-all path-based routing
        │       ├── +page.ts    # Query node by path
        │       └── +page.svelte # Resolve archetype to component
        └── lib/
            ├── raisin.ts                # SDK client singleton + query helpers
            └── components/
                ├── pages/
                │   ├── index.ts         # archetype -> component registry
                │   └── LandingPage.svelte
                └── elements/
                    ├── index.ts         # elementType -> component registry
                    └── Hero.svelte
```

## Package Lifecycle

```bash
# MANDATORY: Validate after every YAML change
raisindb package create ./package --check

# Build .rap file
cd package && raisindb package create .

# Upload to server
raisindb package upload myapp-0.1.0.rap -r myrepo

# Live sync during development (watches for changes)
cd package && raisindb package sync . --watch
```

**RULE**: Run `raisindb package create ./package --check` after every change to any `.yaml` or `.node.yaml` file in `package/`. Never skip this. Fix all errors before moving on.

The `manifest.yaml` declares everything the package provides:

```yaml
name: myapp
version: 0.1.0
provides:
  nodetypes:
    - myapp:Page
  archetypes:
    - myapp:LandingPage
  elementtypes:
    - myapp:Hero
    - myapp:TextBlock
  workspaces:
    - myapp
```

## SDK Connection

Install: `npm install @raisindb/client`

**Important**: Ask the user for their repository name, server URL, and workspace name. Store them in a `.env` file in the frontend directory:

```env
PUBLIC_RAISIN_URL=ws://localhost:8080/sys/default
PUBLIC_RAISIN_REPOSITORY=ask-the-user
PUBLIC_RAISIN_WORKSPACE=my-workspace
```

Default server port is `8080`. The frontend code reads these via `import.meta.env`.

**WebSocket** -- real-time, client-side, supports subscriptions:

```typescript
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

const REPOSITORY = import.meta.env.PUBLIC_RAISIN_REPOSITORY;
const client = new RaisinClient(`${import.meta.env.PUBLIC_RAISIN_URL}/${REPOSITORY}`, {
  tokenStorage: new LocalStorageTokenStorage(REPOSITORY),
  tenantId: 'default',
  defaultBranch: 'main',
});
await client.initSession(REPOSITORY);
const db = client.database(REPOSITORY);
```

**HTTP** -- server-side rendering, SEO pages, no persistent connection:

```typescript
const client = RaisinClient.forSSR(`http://localhost:8080/sys/default/${REPOSITORY}`);
```

**Query helper pattern:**

```typescript
export async function query<T>(sql: string, params?: unknown[]): Promise<T[]> {
  const db = client.database(REPOSITORY);
  const result = await db.executeSql(sql, params);
  return (result.rows ?? []) as T[];
}
```

**Common queries:**

```sql
-- Fetch page by path
SELECT id, path, name, node_type, archetype, properties
FROM myworkspace WHERE path = $1

-- Get children of a path
SELECT * FROM myworkspace WHERE CHILD_OF('/myworkspace/blog')

-- Filter by JSON string property (cast key to String)
SELECT * FROM myworkspace WHERE properties->>'status'::String = 'published'

-- Filter by JSON boolean property (cast key to Boolean)
SELECT * FROM myworkspace WHERE properties->>'featured'::Boolean = true

-- Navigation query: exclude hidden pages
SELECT * FROM myworkspace
WHERE CHILD_OF('/myworkspace') AND node_type = 'myapp:Page'
  AND (properties->>'hide_in_nav'::Boolean != true)

-- Insert a node
INSERT INTO myworkspace (path, node_type, properties)
VALUES ($1, 'myapp:Page', $2::jsonb)
```

## Real-Time Reactivity

RaisinDB pushes events over the WebSocket when nodes change. **This is the standard pattern for all data that can change** — content pages, file uploads, dashboards, precomputed views, navigation. Never use `setTimeout` or polling. Always subscribe to events.

The pattern:
1. **Render current state** immediately (show placeholder/skeleton for data not yet available)
2. **Subscribe** to workspace events via `workspace.events().subscribe()`
3. **Re-fetch and re-render** when events arrive

```typescript
// Subscribe to changes in a folder (or any path pattern)
const db = client.database(REPOSITORY);
const workspace = db.workspace(WORKSPACE_NAME);
const events = workspace.events();

const subscription = await events.subscribe(
  {
    workspace: WORKSPACE_NAME,
    path: '/my-workspace/articles/**',  // glob pattern
    event_types: ['node:created', 'node:updated', 'node:deleted'],
  },
  async (event) => {
    // Re-fetch data when anything changes
    await reloadData();
  }
);

// Clean up on component destroy
onDestroy(() => subscription.unsubscribe());
```

**Event types**: `node:created`, `node:updated`, `node:deleted`, `node:reordered`

**Use cases**:
- File uploads: show skeleton while thumbnail is processing, re-render when `node:updated` fires with the thumbnail
- Content pages: live-update when editors publish changes
- Dashboards: re-render when precomputed summary nodes are rebuilt by triggers
- Navigation: update when pages are added/removed

**DO NOT** use `setTimeout`, `setInterval`, or polling to wait for server-side processing. The WebSocket event will arrive when the data is ready.

## Component Registries

Every RaisinDB frontend needs two registries. Keep them as simple maps.

**Page registry** (`components/pages/index.ts`):

```typescript
export const pageComponents: Record<string, Component<any>> = {
  'myapp:LandingPage': LandingPage,
  'myapp:BlogPost': BlogPost,
};
```

**Element registry** (`components/elements/index.ts`):

```typescript
export const elementComponents: Record<string, Component<any>> = {
  'myapp:Hero': Hero,
  'myapp:TextBlock': TextBlock,
};
```

When you add a new archetype or element type, add a YAML definition in `package/`, create the Svelte/React component, and register it in the corresponding index.

## Server-Side Functions and Triggers

RaisinDB runs JavaScript functions on the server, triggered by events. The runtime includes **built-in image resizing, PDF processing, and AI model access** — no external services needed.

Common uses:
- **File processing**: trigger on asset upload → resize images to thumbnails (`resource.resize()`), extract PDF text (`resource.processDocument()`), store results (`node.addResource()`)
- **Precomputed views**: trigger on content changes → run an aggregation query → store the result as a node. The frontend reads the precomputed node instead of running expensive queries on every page load. Use this for overview lists, dashboards, feeds, statistics, and any data that changes less often than it's read.
- **Business logic**: trigger on content changes → send notifications, validate data, update related nodes
- **AI enrichment**: analyze uploaded images/documents → extract metadata, generate descriptions, tag content

The pattern: define a **trigger** (watches for node events like Created/Updated/Deleted) and a **function** (JavaScript with access to the `raisin.*` API: nodes, SQL, HTTP, AI, binary resources, transactions). Both are YAML + JS files in your RAP package. See `raisindb-functions-triggers` skill.

**Prefer precomputation over real-time queries** for data that is read frequently but changes infrequently. Instead of running a complex SQL query on every page load, have a trigger rebuild a summary node when the source data changes. The frontend then does a simple single-node fetch.

## Learning Path

Read these skills next based on what you need:

- **Model your data** -- `raisindb-content-modeling` (nodetypes, archetypes, elementtypes, properties)
- **Build frontend (Svelte)** -- `raisindb-frontend-sveltekit`
- **Build frontend (React)** -- `raisindb-frontend-react`
- **Query data** -- `raisindb-sql` (SQL syntax, JSON operators, graph queries)
- **Add languages** -- `raisindb-translations` (i18n, locale-aware content)
- **Add auth** -- `raisindb-auth` (login, sessions, user home paths)
- **Handle files** -- `raisindb-file-uploads` (upload, thumbnails via server functions, signed URLs)
- **Server logic** -- `raisindb-functions-triggers` (functions, triggers, event handling)
- **Permissions** -- `raisindb-access-control` (roles, workspace permissions)
