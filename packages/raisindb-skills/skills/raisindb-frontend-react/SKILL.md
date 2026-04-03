---
name: raisindb-frontend-react
description: "Build a React Router frontend for your RaisinDB app with path-based routing, archetype-to-component mapping, and SSR-to-WebSocket upgrade. Use when creating a React frontend."
---

# RaisinDB React Router Frontend

Build a content-driven React app that maps RaisinDB archetypes to React components, uses path-based routing, and upgrades from SSR HTTP to real-time WebSocket after hydration.

## 1. Setup

```bash
npx create-react-router@latest my-app && cd my-app && npm install @raisindb/client
```

**Create `.env`** — ask the user for the repository name and server URL:

```env
VITE_RAISIN_URL=ws://localhost:8080/sys/default
VITE_RAISIN_REPOSITORY=ask-the-user
VITE_RAISIN_WORKSPACE=content
```

The repository is the server-side database name. The workspace is defined in `package/workspaces/*.yaml`. Default port is `8080`.

## 2. TypeScript Types (`lib/types.ts`)

```ts
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
  id: string; path: string; name: string; node_type: string;
  properties: { title: string; slug?: string; order?: number };
}

/** Element fields are flat at root level (no content wrapper) */
export interface Element {
  uuid: string; element_type: string; [key: string]: unknown;
}
```

## 3. Client Singleton (`lib/raisin.ts`)

Module-level singleton with a connection gate so queries wait until `initSession()` completes.

```ts
import {
  RaisinClient, LocalStorageTokenStorage,
  type Database, type IdentityUser,
} from '@raisindb/client';
import type { PageNode, NavItem } from './types';

// Configuration loaded from environment variables
// Create a .env file with these values (ask the user for REPOSITORY):
//   VITE_RAISIN_URL=ws://localhost:8080/sys/default
//   VITE_RAISIN_REPOSITORY=ask-the-user
//   VITE_RAISIN_WORKSPACE=content
const REPOSITORY = import.meta.env.VITE_RAISIN_REPOSITORY || 'CHANGE_ME';
const RAISIN_URL = `${import.meta.env.VITE_RAISIN_URL || 'ws://localhost:8080/sys/default'}/${REPOSITORY}`;
const WORKSPACE = import.meta.env.VITE_RAISIN_WORKSPACE || 'content';

let clientInstance: RaisinClient | null = null;
let dbInstance: Database | null = null;
let connectionPromise: Promise<void> | null = null;
let connectionResolve: (() => void) | null = null;

export function getClient(): RaisinClient {
  if (typeof window === 'undefined') {
    throw new Error('RaisinClient can only be used in the browser');
  }
  if (!clientInstance) {
    clientInstance = new RaisinClient(RAISIN_URL, {
      tokenStorage: new LocalStorageTokenStorage(REPOSITORY),
      tenantId: 'default',
      defaultBranch: 'main',
      connection: { autoReconnect: true, heartbeatInterval: 30000 },
    });
  }
  return clientInstance;
}

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
    connectionResolve?.(); // open gate even on error so queries don't hang
    throw error;
  }
}

export async function login(email: string, password: string): Promise<IdentityUser> {
  return getClient().loginWithEmail(email, password, REPOSITORY);
}

export async function logout(): Promise<void> {
  await getClient().logout();
  dbInstance = null;
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

export async function query<T = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<T[]> {
  const db = await getDatabase();
  const result = await db.executeSql(sql, params);
  return (result.rows ?? []) as T[];
}

export async function queryOne<T = Record<string, unknown>>(sql: string, params?: unknown[]): Promise<T | null> {
  const rows = await query<T>(sql, params);
  return rows[0] ?? null;
}

export async function getPageByPath(path: string): Promise<PageNode | null> {
  const normalized = path.startsWith('/') ? path.slice(1) : path;
  const nodePath = normalized ? `/${WORKSPACE}/${normalized}` : `/${WORKSPACE}`;
  return queryOne<PageNode>(
    `SELECT id, path, name, node_type, archetype, properties FROM ${WORKSPACE} WHERE path = $1 LIMIT 1`,
    [nodePath],
  );
}

export async function getNavigation(): Promise<NavItem[]> {
  try {
    return await query<NavItem>(
      `SELECT id, path, name, node_type, properties FROM ${WORKSPACE}
       WHERE CHILD_OF('/${WORKSPACE}') AND node_type = 'myapp:Page'
         AND (properties->>'hide_in_nav'::Boolean != true)`,
    );
  } catch (error) {
    console.error('[raisin] getNavigation error:', error);
    return [];
  }
}

export { ConnectionState } from '@raisindb/client';
export type { IdentityUser } from '@raisindb/client';
```

`connectionPromise` is the key pattern: `getDatabase()` awaits it, so queries issued before `initSession()` finishes will block rather than fail.

## 4. Page Component Registry

`components/pages/index.ts` -- maps archetype strings to React components:

```ts
import type { ComponentType } from 'react';
import type { PageNode } from '~/lib/types';
import LandingPage from './LandingPage';

export const pageComponents: Record<string, ComponentType<{ page: PageNode }>> = {
  'myapp:LandingPage': LandingPage,
};
```

## 5. Element Component Registry

`components/elements/index.ts` -- maps element type strings to React components:

```ts
import type { ComponentType } from 'react';
import type { Element } from '~/lib/types';
import Hero from './Hero';
import TextBlock from './TextBlock';

export const elementComponents: Record<string, ComponentType<{ element: Element }>> = {
  'myapp:Hero': Hero,
  'myapp:TextBlock': TextBlock,
};
```

## 6. Dynamic Catch-All Route (`routes/$.tsx`)

Splat route that resolves any URL path to a RaisinDB page node, looks up the archetype, and renders the matching page component.

```tsx
import { useState, useEffect } from 'react';
import { useLocation } from 'react-router';
import { getPageByPath } from '~/lib/raisin';
import type { PageNode } from '~/lib/types';
import { pageComponents } from '~/components/pages';

export default function DynamicPage() {
  const location = useLocation();
  const [page, setPage] = useState<PageNode | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    const slug = location.pathname.replace(/^\//, '') || 'home';

    getPageByPath(slug)
      .then((r) => { if (!cancelled) { setPage(r); setLoading(false); } })
      .catch((e) => { if (!cancelled) { setError(e.message); setLoading(false); } });

    return () => { cancelled = true; };
  }, [location.pathname]);

  if (loading) return <div className="p-8 text-center">Loading...</div>;
  if (error) return <div className="p-8 text-red-500">Error: {error}</div>;
  if (!page) return <div className="p-8">Page not found</div>;

  const PageComponent = page.archetype ? pageComponents[page.archetype] : undefined;
  if (!PageComponent) return <div className="p-8">Unknown archetype: {page.archetype}</div>;
  return <PageComponent page={page} />;
}
```

## 7. Page Component Pattern

`LandingPage.tsx` iterates over `properties.content[]` and renders each element through the registry:

```tsx
import type { PageNode } from '~/lib/types';
import { elementComponents } from '~/components/elements';

export default function LandingPage({ page }: { page: PageNode }) {
  const elements = page.properties.content ?? [];
  return (
    <article>
      <h1 className="text-3xl font-bold mb-6">{page.properties.title}</h1>
      {elements.map((el) => {
        const C = elementComponents[el.element_type];
        return C
          ? <C key={el.uuid} element={el} />
          : <div key={el.uuid} className="p-4 bg-yellow-50">Unknown: {el.element_type}</div>;
      })}
    </article>
  );
}
```

## 8. Element Component Pattern

`Hero.tsx` -- element fields are flat on the element object (no `content` wrapper):

```tsx
import type { Element } from '~/lib/types';

export default function Hero({ element }: { element: Element }) {
  const { headline, subheadline, cta_text, cta_link } = element as Record<string, any>;
  return (
    <section className="py-20 px-8 text-center bg-gradient-to-r from-blue-600 to-purple-600 text-white">
      {headline && <h1 className="text-5xl font-bold mb-4">{headline}</h1>}
      {subheadline && <p className="text-xl opacity-90 mb-8">{subheadline}</p>}
      {cta_text && cta_link && (
        <a href={cta_link} className="px-6 py-3 bg-white text-blue-600 rounded-lg font-semibold">
          {cta_text}
        </a>
      )}
    </section>
  );
}
```

## 9. Root Layout with Auth Init

`root.tsx` initializes the session once on mount and provides auth state via context. The connection gate in `raisin.ts` ensures child routes' `getPageByPath()` calls automatically wait.

```tsx
import { createContext, useContext, useState, useEffect } from 'react';
import { Outlet } from 'react-router';
import { initSession, type IdentityUser } from '~/lib/raisin';

interface AuthCtxType { user: IdentityUser | null; loading: boolean }
const AuthCtx = createContext<AuthCtxType>({ user: null, loading: true });
export const useAuthContext = () => useContext(AuthCtx);

export default function RootLayout() {
  const [user, setUser] = useState<IdentityUser | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    initSession()
      .then((u) => { setUser(u); setLoading(false); })
      .catch(() => setLoading(false));
  }, []);

  return (
    <AuthCtx.Provider value={{ user, loading }}>
      <nav>{/* navigation */}</nav>
      <main><Outlet /></main>
    </AuthCtx.Provider>
  );
}
```

## 10. SSR with `forSSR` and the Hybrid Client

This is the key React differentiator: server loaders fetch via HTTP, the client hydrates, then upgrades to WebSocket for real-time updates.

### SSR Config (`lib/config.ts`)

```ts
import type { SSRClientConfig } from '@raisindb/client';
export const REPOSITORY = import.meta.env.VITE_RAISIN_REPOSITORY || 'CHANGE_ME';
export const WORKSPACE = import.meta.env.VITE_RAISIN_WORKSPACE || 'content';

export function getRaisinConfig(): SSRClientConfig {
  const s = typeof window === 'undefined';
  return {
    httpBaseUrl: s ? (process.env.RAISIN_HTTP_URL || 'http://localhost:8080')
                   : (window.ENV?.RAISIN_HTTP_URL || 'http://localhost:8080'),
    wsUrl: s ? (process.env.RAISIN_WS_URL || 'ws://localhost:8080/sys/default')
             : (window.ENV?.RAISIN_WS_URL || 'ws://localhost:8080/sys/default'),
    httpOptions: { tenantId: 'default', defaultBranch: 'main' },
    wsOptions:   { tenantId: 'default', defaultBranch: 'main' },
  };
}
```

### Server Loader (`createLoader` creates an HTTP client, runs your callback, returns serializable data)

```tsx
import { createLoader, rowsToObjects } from '@raisindb/client';
import { getRaisinConfig, REPOSITORY, WORKSPACE } from '~/lib/config';

export const loader = createLoader(getRaisinConfig(), async (client) => {
  const db = client.database(REPOSITORY);
  const result = await db.executeSql(
    `SELECT * FROM ${WORKSPACE} WHERE node_type = 'myapp:Post' ORDER BY created_at DESC LIMIT 50`,
  );
  return { posts: rowsToObjects(result.columns, result.rows) };
});
```

### Hybrid Client Hook (`hooks/useHybridClient.ts`)

After hydration, upgrades from HTTP to WebSocket for real-time subscriptions:

```ts
import { useState, useEffect, useRef } from 'react';
import { RaisinClient, RaisinHttpClient, ConnectionState } from '@raisindb/client';
import { getRaisinConfig } from '~/lib/config';

export type ClientMode = 'http' | 'websocket' | 'connecting' | 'error';

export function useHybridClient(autoUpgrade = true) {
  const config = getRaisinConfig();
  const [httpClient] = useState(() => RaisinClient.forSSR(config.httpBaseUrl, config.httpOptions));
  const [wsClient, setWsClient] = useState<RaisinClient | null>(null);
  const [connState, setConnState] = useState(ConnectionState.Disconnected);
  const [error, setError] = useState<string | null>(null);
  const hasUpgraded = useRef(false);

  const upgrade = async () => {
    if (hasUpgraded.current) return;
    hasUpgraded.current = true;
    try {
      const client = new RaisinClient(config.wsUrl, config.wsOptions);
      client.on('stateChange', (s: ConnectionState) => setConnState(s));
      client.on('error', (e: Error) => setError(e.message));
      await client.connect();
      if (config.credentials) await client.authenticate(config.credentials);
      setWsClient(client);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Connection failed');
      hasUpgraded.current = false;
    }
  };

  const reconnect = () => {
    hasUpgraded.current = false;
    setError(null);
    wsClient?.disconnect();
    setWsClient(null);
    upgrade();
  };

  useEffect(() => {
    if (autoUpgrade && typeof window !== 'undefined') upgrade();
    return () => { wsClient?.disconnect(); };
  }, [autoUpgrade]);

  let mode: ClientMode = 'http';
  if (error && !wsClient) mode = 'error';
  else if (connState === ConnectionState.Connecting) mode = 'connecting';
  else if (wsClient && connState === ConnectionState.Connected) mode = 'websocket';

  return { mode, httpClient, wsClient, isRealtime: mode === 'websocket', error, reconnect };
}
```

### Combining SSR + Real-Time

In your route component, seed state from `loaderData`, then subscribe after WebSocket upgrade:

```tsx
export default function Feed({ loaderData }: Route.ComponentProps) {
  const [posts, setPosts] = useState(loaderData.posts);
  const { wsClient, isRealtime } = useHybridClient();

  useEffect(() => {
    if (!isRealtime || !wsClient) return;
    const sub = wsClient.database(REPOSITORY).events().subscribe(
      { workspace: WORKSPACE, node_type: 'Post' },
      (event) => { /* re-fetch or append to posts state */ },
    );
    return () => sub.unsubscribe();
  }, [isRealtime, wsClient]);

  return <ul>{posts.map((p: any) => <li key={p.id}>{p.name}</li>)}</ul>;
}
```

SSR flow: server runs `loader` via HTTP (fully rendered for SEO) -> React hydrates with `loaderData` -> `useHybridClient()` opens WebSocket -> once `isRealtime`, subscriptions start and UI updates live.

## 11. Real-Time Updates

Components can subscribe to content changes via WebSocket events. Use this for live-updating dashboards, file browsers, or any data that changes while the user views it. See `raisindb-overview` for the full pattern.

```tsx
function FileList({ folderPath }: { folderPath: string }) {
  const [items, setItems] = useState<Asset[]>([]);

  useEffect(() => {
    loadFiles();
    const db = getDatabase();
    const ws = db.workspace(WORKSPACE);
    let sub: any;
    ws.events().subscribe(
      { workspace: WORKSPACE, path: folderPath + '/**',
        event_types: ['node:created', 'node:updated', 'node:deleted'] },
      () => loadFiles()
    ).then(s => { sub = s; });
    return () => sub?.unsubscribe();
  }, [folderPath]);
}
```

Never use `setTimeout` or polling to wait for server-side processing (thumbnails, precomputed views, etc.). Subscribe to events — the WebSocket pushes updates when data is ready.

## 12. React Provider Alternative (Client-Only)

For apps without SSR, use `createRaisinReact` instead of the singleton pattern:

```ts
// lib/raisin-hooks.ts
import React from 'react';
import { RaisinClient, LocalStorageTokenStorage, createRaisinReact } from '@raisindb/client';

export const client = new RaisinClient('wss://localhost:8443/sys/default/myapp', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
  tenantId: 'default', defaultBranch: 'main',
});
export const { RaisinProvider, useAuth, useConnection, useSql, useSubscription } = createRaisinReact(React);
```

Wrap your app with `<RaisinProvider client={client} repository="myapp">`, then use hooks:

```tsx
function PostList() {
  const { data: posts } = useSql<Post>(
    "SELECT * FROM content WHERE node_type = 'myapp:Post' ORDER BY created_at DESC",
    [], { realtime: { workspace: 'content', nodeType: 'Post' } },
  );
  return posts?.map((p) => <div key={p.id}>{p.name}</div>);
}
```

`useSql` with `realtime` handles event subscription and auto-refetch internally. Use **singleton** (sections 3-9) for full control with SSR loaders, or **provider + hooks** for less boilerplate in client-only apps.
