/**
 * RaisinDB client singleton + React hooks for the TaskPilot demo.
 *
 * This is the canonical `createRaisinReact` wiring: one RaisinClient, the
 * hooks created once by passing the real React module ("bring your own
 * framework" — the SDK never imports react).
 *
 * Connection URL: tenant-less `ws(s)://host/ws/{repository}` — no tenantId
 * option; the server resolves the tenant from the route.
 */
import * as React from 'react';
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';
import { createRaisinReact } from '@raisindb/client/react';

const isBrowser = typeof window !== 'undefined';

/** Repository name (last path segment of WS_URL by convention). */
export const REPOSITORY: string = import.meta.env.VITE_RAISIN_REPO ?? 'taskpilot';

export const WS_URL: string =
  import.meta.env.VITE_RAISIN_WS_URL ?? `ws://localhost:8081/ws/${REPOSITORY}`;

/** Path of the AI agent we chat with (lives in the `functions` workspace). */
export const AGENT_PATH = '/agents/pilot';

export const client = new RaisinClient(WS_URL, {
  tokenStorage: isBrowser ? new LocalStorageTokenStorage('taskpilot') : undefined,
  connection: { autoReconnect: true, heartbeatInterval: 30000 },
  requestTimeout: 30000,
  // HTTP (login, conversations SSE, SQL fallback) goes same-origin through
  // the Vite dev proxy (see vite.config.ts) so no CORS entry is needed on
  // the server. Override with VITE_RAISIN_HTTP_URL to talk directly.
  httpBaseUrl:
    import.meta.env.VITE_RAISIN_HTTP_URL ??
    (isBrowser ? window.location.origin : 'http://localhost:8081'),
});

export const {
  RaisinProvider,
  useRaisinClient,
  useDatabase,
  useAuth,
  useConnection,
  useSql,
  useSubscription,
  useConversation,
  useConversationList,
} = createRaisinReact(React);
