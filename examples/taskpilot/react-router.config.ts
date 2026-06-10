import type { Config } from '@react-router/dev/config';

// SPA mode: the RaisinDB client is a browser WebSocket client, so the app is
// client-rendered. React Router 7 framework mode still gives us file routes,
// typegen, and the production build.
export default {
  ssr: false,
} satisfies Config;
