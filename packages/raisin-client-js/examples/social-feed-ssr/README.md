# Social Feed SSR Example

A server-side rendered (SSR) social feed application built with **React Router 7**, **RaisinDB**, and **Tailwind CSS**. This example demonstrates how to build a modern web application with:

- ⚡ **Server-Side Rendering** for fast initial page loads and SEO
- 🔄 **Real-time Updates** via WebSocket after hydration
- 📊 **Hybrid Data Fetching** (HTTP for SSR, WebSocket for real-time)
- 🎨 **Glassmorphism UI** with Tailwind CSS
- ♿ **Progressive Enhancement** - works without JavaScript

## Architecture

```
Server (SSR)          Client (Hydration)       Client (Real-time)
     ↓                        ↓                        ↓
HTTP API (Loader)    →   React Hydration   →   WebSocket Upgrade
  - Fast initial          - Interactive          - Live updates
  - SEO-friendly          - Same data            - Subscriptions
```

### Key Features

1. **Server-Side Rendering**
   - Initial page render happens on the server
   - Data fetched via HTTP REST API during SSR
   - Full HTML sent to browser (works without JS)

2. **Client Hydration**
   - React attaches to server-rendered HTML
   - Uses same data from server (no flash)
   - Interactive immediately after hydration

3. **Real-Time Upgrade**
   - After hydration, establishes WebSocket connection
   - Subscribes to real-time database events
   - New posts appear instantly without refresh

## Getting Started

### Prerequisites

- Node.js 18+ (for native fetch support)
- Running RaisinDB server on `localhost:8080`
- Database initialized with social feed schema

### Installation

```bash
# Install dependencies
npm install

# Initialize database (if needed)
npm run init-db

# Start development server
npm run dev
```

The app will be available at [http://localhost:5173](http://localhost:5173)

### Production Build

```bash
# Build for production
npm run build

# Start production server
npm start
```

## Project Structure

```
social-feed-ssr/
├── app/
│   ├── routes/              # React Router 7 routes
│   │   └── _index.tsx       # Feed route with loader
│   ├── components/          # React components
│   │   ├── PostCard.tsx
│   │   ├── CreatePost.tsx
│   │   ├── LiveIndicator.tsx
│   │   └── ...
│   ├── hooks/               # Custom React hooks
│   │   └── useHybridClient.ts  # HTTP→WebSocket upgrade
│   ├── lib/                 # Utilities and config
│   │   ├── config.ts        # Client configuration
│   │   ├── types.ts         # TypeScript types
│   │   └── utils.ts         # Helper functions
│   ├── entry.server.tsx     # SSR entry point
│   ├── entry.client.tsx     # Client hydration entry
│   └── root.tsx             # Root layout
├── public/                  # Static assets
├── react-router.config.ts   # React Router 7 config
├── package.json
└── README.md
```

## How It Works

### 1. Server-Side Rendering (SSR)

When a user first visits the page, the server:

1. Receives the HTTP request
2. Creates an HTTP client (`RaisinHttpClient`)
3. Runs the `loader` function to fetch data
4. Renders React components to HTML
5. Sends complete HTML to browser

**Example Loader** (`app/routes/_index.tsx`):

```typescript
export const loader = createLoader(
  getRaisinConfig(),
  async (client) => {
    const db = client.database('social');

    // Fetch posts via HTTP during SSR
    const result = await db.executeSql(`
      SELECT * FROM social WHERE node_type = 'Post'
      ORDER BY created_at DESC LIMIT 50
    `);

    return { posts: rowsToObjects(result.columns, result.rows) };
  }
);
```

### 2. Client Hydration

After HTML arrives in the browser:

1. React hydrates the server-rendered HTML
2. Components become interactive
3. `useHybridClient` hook initializes

**Hydration** (`app/entry.client.tsx`):

```typescript
hydrateRoot(
  document,
  <StrictMode>
    <HydratedRouter />
  </StrictMode>
);
```

### 3. WebSocket Upgrade

The `useHybridClient` hook automatically:

1. Detects client-side environment
2. Establishes WebSocket connection
3. Authenticates with credentials
4. Provides WebSocket client to components

**Hybrid Client Hook** (`app/hooks/useHybridClient.ts`):

```typescript
export function useHybridClient() {
  const [wsClient, setWsClient] = useState<RaisinClient | null>(null);

  useEffect(() => {
    const client = new RaisinClient(config.wsUrl);
    await client.connect();
    await client.authenticate(credentials);
    setWsClient(client);
  }, []);

  return { wsClient, isRealtime: !!wsClient };
}
```

### 4. Real-Time Updates

Once WebSocket is connected, components can subscribe to events:

```typescript
useEffect(() => {
  if (!isRealtime || !wsClient) return;

  const subscription = wsClient
    .database('social')
    .events()
    .subscribe(
      { workspace: 'default', node_type: 'Post' },
      (event) => {
        if (event.event_type === 'node:created') {
          // Add new post to feed
        }
      }
    );

  return () => subscription.unsubscribe();
}, [isRealtime, wsClient]);
```

## Environment Configuration

Create a `.env` file to customize the configuration:

```env
RAISIN_HTTP_URL=http://localhost:8080
RAISIN_WS_URL=ws://localhost:8080/sys/default
```

Or modify `app/lib/config.ts` directly.

## Comparison with Client-Only Version

| Feature | Client-Only (`social-feed`) | SSR (`social-feed-ssr`) |
|---------|----------------------------|-------------------------|
| **Initial Render** | Client-side (blank → data) | Server-side (complete HTML) |
| **SEO** | Limited (SPA) | Full (HTML from server) |
| **First Paint** | Slower (JS bundle + fetch) | Faster (HTML ready) |
| **Interactivity** | After JS + fetch | After hydration |
| **Real-time** | WebSocket from start | After upgrade |
| **No-JS Support** | ❌ Does not work | ✅ Static content works |

## Differences from Original Social Feed

1. **Routing**: Uses React Router 7 with SSR instead of client-side routing
2. **Data Fetching**: HTTP loaders for SSR, then WebSocket for real-time
3. **Client Hook**: `useHybridClient` instead of `useRaisinClient`
4. **Entry Points**: Separate server and client entries
5. **Configuration**: Unified config for both HTTP and WebSocket

## Common Patterns

### Creating a New Route with SSR

```typescript
// app/routes/my-route.tsx
import { createLoader } from '@raisindb/client';
import { getRaisinConfig } from '~/lib/config';

export const loader = createLoader(
  getRaisinConfig(),
  async (client) => {
    // Fetch data via HTTP during SSR
    const data = await client.database('my-db').executeSql('...');
    return { data };
  }
);

export default function MyRoute() {
  const { data } = useLoaderData();

  // Component code...
}
```

### Adding Real-Time Subscriptions

```typescript
const { wsClient, isRealtime } = useHybridClient();

useEffect(() => {
  if (!isRealtime || !wsClient) return;

  const subscription = wsClient
    .database('my-db')
    .events()
    .subscribe(filters, callback);

  return () => subscription.unsubscribe();
}, [isRealtime, wsClient]);
```

## Troubleshooting

### "Failed to connect" error

- Ensure RaisinDB server is running on `localhost:8080`
- Check firewall settings
- Verify credentials in `app/lib/config.ts`

### WebSocket not upgrading

- Check browser console for errors
- Verify `RAISIN_WS_URL` is correct
- Ensure authentication is working

### Hydration mismatch

- Ensure data format is consistent between server and client
- Check for browser-only code running during SSR
- Verify `useEffect` is used for client-only code

## Learn More

- [React Router 7 Documentation](https://reactrouter.com)
- [RaisinDB Client SDK](../../README.md)
- [Original Social Feed Example](../social-feed)

## License

MIT
