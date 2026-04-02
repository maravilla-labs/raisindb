---
name: raisindb-frontend-sveltekit
description: "Build a SvelteKit frontend for your RaisinDB app with path-based routing, archetype-to-component mapping, and real-time updates. Use when creating a SvelteKit frontend."
---

# RaisinDB SvelteKit Frontend

Build a SvelteKit app that renders RaisinDB content pages using path-based routing. Pages are fetched by path via SQL over WebSocket, then rendered by mapping their archetype to a Svelte component. Elements inside pages use the same registry pattern.

## 1. Setup

```bash
npm create svelte@latest frontend   # choose Skeleton, TypeScript
cd frontend
npm install @raisindb/client
```

Use `adapter-auto` (default) or `adapter-node`. SSR is disabled since all data comes over WebSocket.

**Create `frontend/.env`** — ask the user for the repository name and server URL:

```env
PUBLIC_RAISIN_URL=ws://localhost:8080/sys/default
PUBLIC_RAISIN_REPOSITORY=ask-the-user
PUBLIC_RAISIN_WORKSPACE=my-workspace
```

The repository is the server-side database name. The workspace is defined in `package/workspaces/*.yaml`. The URL default port is `8080`.

## 2. Client Singleton (`src/lib/raisin.ts`)

One file handles the client instance, connection gate, SQL helpers, page lookup, navigation, asset URLs, and type definitions.

```typescript
import {
  RaisinClient,
  LocalStorageTokenStorage,
  type IdentityUser,
  type Database,
} from '@raisindb/client';
import { browser } from '$app/environment';

// Configuration loaded from environment variables
// Create a .env file in frontend/ with these values (ask the user for REPOSITORY):
//   PUBLIC_RAISIN_URL=ws://localhost:8080/sys/default
//   PUBLIC_RAISIN_REPOSITORY=ask-the-user
//   PUBLIC_RAISIN_WORKSPACE=my-workspace
const REPOSITORY = import.meta.env.PUBLIC_RAISIN_REPOSITORY || 'CHANGE_ME';
const RAISIN_URL = `${import.meta.env.PUBLIC_RAISIN_URL || 'ws://localhost:8080/sys/default'}/${REPOSITORY}`;
const TENANT_ID = 'default';
const WORKSPACE_NAME = import.meta.env.PUBLIC_RAISIN_WORKSPACE || 'my-workspace';

let clientInstance: RaisinClient | null = null;
let dbInstance: Database | null = null;
let connectionPromise: Promise<void> | null = null;
let connectionResolve: (() => void) | null = null;

export function getClient(): RaisinClient {
  if (!browser) throw new Error('RaisinClient can only be used in the browser');
  if (!clientInstance) {
    clientInstance = new RaisinClient(RAISIN_URL, {
      tokenStorage: new LocalStorageTokenStorage(REPOSITORY),
      tenantId: TENANT_ID,
      defaultBranch: 'main',
      connection: { autoReconnect: true, heartbeatInterval: 30000 },
      requestTimeout: 30000,
    });
  }
  return clientInstance;
}

/** MUST be called before any queries. Creates a connection gate. */
export async function initSession(): Promise<IdentityUser | null> {
  if (!connectionPromise) {
    connectionPromise = new Promise((resolve) => { connectionResolve = resolve; });
  }
  const client = getClient();
  try {
    const user = await client.initSession(REPOSITORY);
    if (!user && !client.isConnected()) await client.connect();
    connectionResolve?.();
    return user;
  } catch (error) {
    connectionResolve?.();
    throw error;
  }
}

export async function getDatabase(): Promise<Database> {
  if (connectionPromise) await connectionPromise;
  if (!dbInstance) {
    const client = getClient();
    if (!client.isConnected()) await client.connect();
    dbInstance = client.database(REPOSITORY);
  }
  return dbInstance;
}

export async function query<T = Record<string, unknown>>(
  sql: string, params?: unknown[]
): Promise<T[]> {
  const db = await getDatabase();
  const result = await db.executeSql(sql, params);
  return (result.rows ?? []) as T[];
}

export async function queryOne<T = Record<string, unknown>>(
  sql: string, params?: unknown[]
): Promise<T | null> {
  const rows = await query<T>(sql, params);
  return rows[0] ?? null;
}

/** Fetch a page by URL path. Maps slug to workspace-prefixed node path. */
export async function getPageByPath(path: string): Promise<PageNode | null> {
  const normalizedPath = path.startsWith('/') ? path.slice(1) : path;
  const nodePath = normalizedPath
    ? `/${WORKSPACE_NAME}/${normalizedPath}`
    : `/${WORKSPACE_NAME}`;

  return queryOne<PageNode>(`
    SELECT id, path, name, node_type, archetype, properties
    FROM ${WORKSPACE_NAME}
    WHERE path = $1
    LIMIT 1
  `, [nodePath]);
}

/** Fetch nav items -- direct children of workspace root. */
export async function getNavigation(): Promise<NavItem[]> {
  try {
    return await query<NavItem>(`
      SELECT id, path, name, node_type, properties
      FROM ${WORKSPACE_NAME}
      WHERE CHILD_OF('/${WORKSPACE_NAME}')
        AND node_type = 'my-app:Page'
        AND (properties->>'hide_in_nav'::Boolean != true)
    `);
  } catch (error) {
    console.error('[raisin] getNavigation error:', error);
    return [];
  }
}

/** Get a signed URL for displaying or downloading an asset. */
export async function signAssetUrl(
  nodePath: string,
  command: 'display' | 'download' = 'display',
  options?: { propertyPath?: string }
): Promise<{ url: string }> {
  const db = await getDatabase();
  const ws = db.workspace(WORKSPACE_NAME);
  return ws.signAssetUrl(nodePath, command, options);
}

// --- Types ---

export interface PageNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  archetype?: string;
  properties: {
    title: string;
    slug?: string;
    description?: string;
    order?: number;
    content?: Element[];
  };
}

export interface NavItem {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: { title: string; slug?: string; order?: number };
}

export interface Element {
  uuid: string;
  element_type: string;
  [key: string]: unknown; // element fields sit at root level (flat format)
}
```

Key points:
- **Singleton** avoids duplicate WebSocket connections.
- **Connection gate** -- `query()` awaits `connectionPromise`, so page loaders can call it before `initSession()` finishes.
- **Path mapping** -- URL `/events/summer-fest` maps to node path `/${WORKSPACE_NAME}/events/summer-fest`.
- **Navigation** -- `CHILD_OF('/${WORKSPACE_NAME}')` returns direct children; filter by `node_type` for pages.

## 3. Page Component Registry (`src/lib/components/pages/index.ts`)

Map archetype names to Svelte page components. The `archetype` field on a `PageNode` selects which component renders it.

```typescript
import type { Component } from 'svelte';
import LandingPage from './LandingPage.svelte';
import type { PageNode } from '$lib/raisin';

export const pageComponents: Record<string, Component<any>> = {
  'my-app:LandingPage': LandingPage,
  // 'my-app:BlogPost': BlogPost,
};

export function getPageComponent(
  archetype: string | undefined
): Component<{ page: PageNode }> | undefined {
  if (!archetype) return undefined;
  return pageComponents[archetype];
}
```

## 4. Element Component Registry (`src/lib/components/elements/index.ts`)

Map element type names to Svelte element components. Pages iterate `properties.content[]` and render each element via this registry.

```typescript
import type { Component } from 'svelte';
import Hero from './Hero.svelte';
import TextBlock from './TextBlock.svelte';

export const elementComponents: Record<string, Component<any>> = {
  'my-app:Hero': Hero,
  'my-app:TextBlock': TextBlock,
};

export function getElementComponent(
  elementType: string
): Component<{ element: Record<string, unknown> }> | undefined {
  return elementComponents[elementType];
}
```

## 5. Root Layout (`src/routes/+layout.ts`)

Initializes the session and loads navigation before any page renders.

```typescript
import type { LayoutLoad } from './$types';
import { browser } from '$app/environment';
import { initSession, getNavigation, type IdentityUser, type NavItem } from '$lib/raisin';

export const ssr = false;
export const prerender = false;

export const load: LayoutLoad = async () => {
  if (!browser) {
    return { user: null as IdentityUser | null, navigationItems: [] as NavItem[] };
  }
  try {
    const user = await initSession();
    const navigationItems = await getNavigation();
    return { user, navigationItems };
  } catch (e) {
    console.error('[layout] Init failed:', e);
    return {
      user: null, navigationItems: [],
      error: e instanceof Error ? e.message : 'Failed to connect',
    };
  }
};
```

## 6. Root Layout Component (`src/routes/+layout.svelte`)

Renders navigation from loaded data and a slot for page content.

```svelte
<script lang="ts">
  import { page } from '$app/stores';
  import type { LayoutData } from './$types';

  interface Props {
    data: LayoutData;
    children: import('svelte').Snippet;
  }
  let { data, children }: Props = $props();
</script>

<nav>
  <a href="/">Home</a>
  {#each data.navigationItems as item}
    {@const slug = item.properties.slug || item.name}
    <a href="/{slug}" class:active={$page.url.pathname === `/${slug}`}>
      {item.properties.title || item.name}
    </a>
  {/each}
</nav>

<main>
  {#if data.error}
    <p>Connection error: {data.error}</p>
  {:else}
    {@render children()}
  {/if}
</main>
```

## 7. Dynamic Route Loader (`src/routes/[...slug]/+page.ts`)

Catch-all route that loads a page by URL slug. Defaults to `home` for the root path.

```typescript
import type { PageLoad } from './$types';
import { getPageByPath } from '$lib/raisin';

export const load: PageLoad = async ({ params }) => {
  const slug = params.slug || 'home';
  const path = `/${slug}`;

  try {
    const page = await getPageByPath(path);
    return { page };
  } catch (error) {
    console.error(`Failed to load page: ${path}`, error);
    return { page: null, error: error instanceof Error ? error.message : 'Page not found' };
  }
};
```

## 8. Page Renderer (`src/routes/[...slug]/+page.svelte`)

Looks up the archetype in the page registry. Falls back to not-found or a debug view.

```svelte
<script lang="ts">
  import { pageComponents } from '$lib/components/pages';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  const PageComponent = $derived(
    data.page?.archetype ? pageComponents[data.page.archetype] : undefined
  );
</script>

<svelte:head>
  {#if data.page}
    <title>{data.page.properties.title}</title>
    {#if data.page.properties.description}
      <meta name="description" content={data.page.properties.description} />
    {/if}
  {:else}
    <title>Page Not Found</title>
  {/if}
</svelte:head>

{#if data.error}
  <div class="error-page">
    <h1>Error</h1>
    <p>{data.error}</p>
    <a href="/">Go Home</a>
  </div>
{:else if !data.page}
  <div class="not-found">
    <h1>Page Not Found</h1>
    <a href="/">Go Home</a>
  </div>
{:else if PageComponent}
  <PageComponent page={data.page} />
{:else}
  <div class="no-template">
    <h1>{data.page.properties.title}</h1>
    <p>No template for archetype: {data.page.archetype || 'none'}</p>
    <pre>{JSON.stringify(data.page, null, 2)}</pre>
  </div>
{/if}
```

## 9. Page Component Pattern (`LandingPage.svelte`)

A page component receives the full `PageNode`, iterates `properties.content[]`, and renders each element via the element registry.

```svelte
<script lang="ts">
  import { elementComponents } from '$lib/components/elements';
  import type { PageNode, Element } from '$lib/raisin';

  interface Props { page: PageNode; }
  let { page }: Props = $props();

  const elements: Element[] = $derived(page.properties.content ?? []);
</script>

<article>
  {#each elements as element (element.uuid)}
    {@const Component = elementComponents[element.element_type]}
    {#if Component}
      <Component {element} />
    {:else}
      <div class="unknown-element">Unknown element type: {element.element_type}</div>
    {/if}
  {/each}
</article>
```

## 10. Element Component Pattern (`Hero.svelte`)

An element component receives a single element object. Fields from the element type definition are available directly on the element (flat format -- no content wrapper).

```svelte
<script lang="ts">
  import type { Element } from '$lib/raisin';

  interface HeroElement extends Element {
    headline?: string;
    subheadline?: string;
    cta_text?: string;
    cta_link?: string;
    background_image?: { url?: string };
  }

  interface Props { element: HeroElement; }
  let { element }: Props = $props();
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

## Data Flow Summary

```
URL slug -> getPageByPath() -> PageNode -> archetype lookup -> Page component
                                              |
                                    iterate content[] -> element type lookup -> Element component
```

| File | Role |
|------|------|
| `src/lib/raisin.ts` | Client singleton, connection gate, SQL helpers, `getPageByPath`, `getNavigation` |
| `src/lib/components/pages/index.ts` | Archetype name to page component map |
| `src/lib/components/elements/index.ts` | Element type name to element component map |
| `src/routes/+layout.ts` | `initSession()`, load nav, `ssr = false` |
| `src/routes/+layout.svelte` | Render nav + page slot |
| `src/routes/[...slug]/+page.ts` | Call `getPageByPath(slug)` |
| `src/routes/[...slug]/+page.svelte` | Archetype lookup, render component or not-found |
| `src/lib/components/pages/LandingPage.svelte` | Iterate `content[]`, render elements |
| `src/lib/components/elements/Hero.svelte` | Render element fields |
