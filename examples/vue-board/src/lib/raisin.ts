/**
 * RaisinDB client singleton + Vue composables for the vue-board demo.
 *
 * This is the canonical `createRaisinVue` wiring: one RaisinClient, one
 * Database handle, and the composables created once by passing the real
 * Vue module ("bring your own framework" — the SDK never imports vue).
 *
 * Connection URL: tenant-less `ws(s)://host/ws/{repository}` — no tenantId
 * option; the server resolves the tenant from the route.
 */
import * as vue from 'vue';
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';
import { createRaisinVue } from '@raisindb/client/vue';

export const WS_URL: string =
  import.meta.env.VITE_RAISIN_WS_URL ?? 'ws://localhost:8081/ws/shiftboard2';

/** Repository name (last path segment of WS_URL by convention). */
export const REPOSITORY: string = import.meta.env.VITE_RAISIN_REPO ?? 'shiftboard2';

/** Path of the AI agent we chat with (lives in the `functions` workspace). */
export const AGENT_PATH = '/agents/shift-planner';

export const client = new RaisinClient(WS_URL, {
  tokenStorage: new LocalStorageTokenStorage('vue-board'),
  connection: { autoReconnect: true, heartbeatInterval: 30000 },
  requestTimeout: 30000,
  // HTTP (login, conversations SSE) goes same-origin through the Vite dev
  // proxy (see vite.config.ts) so no CORS entry is needed on the server.
  // Override with VITE_RAISIN_HTTP_URL to talk to the server directly.
  httpBaseUrl: import.meta.env.VITE_RAISIN_HTTP_URL ?? window.location.origin,
});

/**
 * Database handle for SQL, node events and conversations. Created via
 * client.database() so it carries the HTTP context the conversations
 * (SSE chat) API needs.
 */
export const db = client.database(REPOSITORY);

export const {
  useAuth,
  useConnection,
  useSql,
  useSubscription,
  useConversation,
  useConversationList,
} = createRaisinVue(vue);
