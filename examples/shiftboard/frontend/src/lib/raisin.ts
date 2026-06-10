/**
 * Browser-side RaisinDB WebSocket client for the Shiftboard demo.
 *
 * The session is owned by the server (httpOnly cookies, see
 * src/lib/server/raisin.ts); the layout load hands the access token to the
 * page, and `initClient(token)` authenticates the WebSocket with
 * `{ type: 'jwt' }`. The token is also seeded into the client's token storage
 * so the SDK can re-authenticate after reconnects and attach the bearer to
 * its HTTP/SSE calls (conversations API).
 */
import {
  AuthManager,
  InboxApi,
  MemoryTokenStorage,
  RaisinClient,
  type Database,
} from '@raisindb/client';

/**
 * WebSocket endpoint: ws(s)://host/ws/{repository} (tenant-less).
 * Operators with multiple tenants can use ws(s)://host/sys/{tenant}/{repository}.
 */
export const WS_URL =
  import.meta.env.VITE_RAISIN_WS_URL ?? 'ws://localhost:8081/ws/shiftboard';

/** Repository name (last path segment of WS_URL by convention). */
export const REPOSITORY = import.meta.env.VITE_RAISIN_REPO ?? 'shiftboard';

/** ws(s)://host/ws/{repo} → http(s)://host (for the inbox REST API). */
function httpBase(): string {
  const url = new URL(WS_URL);
  url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
  return url.origin;
}

let client: RaisinClient | null = null;
let db: Database | null = null;
let tokenStorage: MemoryTokenStorage | null = null;
let inbox: InboxApi | null = null;

/** Connect + authenticate the singleton WS client. Call once after hydration. */
export async function initClient(accessToken: string): Promise<RaisinClient> {
  if (client) return client;

  const storage = new MemoryTokenStorage();
  storage.setAccessToken(accessToken);
  tokenStorage = storage;

  client = new RaisinClient(WS_URL, {
    tokenStorage: storage,
    tenantId: 'default',
    defaultBranch: 'main',
    connection: { autoReconnect: true, heartbeatInterval: 30000 },
    requestTimeout: 30000,
  });

  await client.connect();
  await client.authenticate({ type: 'jwt', token: accessToken });
  return client;
}

export function getClient(): RaisinClient {
  if (!client) throw new Error('RaisinDB client not initialized — call initClient() first');
  return client;
}

/**
 * Database handle for SQL, events and conversations.
 * Created via client.database() so it carries the HTTP context that
 * db.conversations (SSE chat API) needs.
 */
export function getDb(): Database {
  if (!db) {
    db = getClient().database(REPOSITORY);
  }
  return db;
}

/**
 * Inbox task API (REST): list the logged-in user's human tasks and complete
 * them. Shares the WS client's token storage, so the bearer follows token
 * refreshes after reconnects.
 */
export function getInbox(): InboxApi {
  if (!inbox) {
    if (!tokenStorage) {
      throw new Error('RaisinDB client not initialized — call initClient() first');
    }
    inbox = new InboxApi(httpBase(), REPOSITORY, new AuthManager(tokenStorage));
  }
  return inbox;
}
