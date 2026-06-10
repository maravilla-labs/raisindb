import { reactRouter } from '@react-router/dev/vite';
import { defineConfig } from 'vite';

// The WebSocket connects straight to the server (not CORS-restricted), but
// the SDK's HTTP calls (login, conversations SSE, SQL-over-HTTP) are
// same-origin via this dev proxy, so the demo needs no CORS entry on the
// server. Override with VITE_RAISIN_HTTP_URL to talk to the server directly.
const RAISIN_HTTP = process.env.VITE_RAISIN_HTTP_URL ?? 'http://localhost:8081';

const proxy = {
  '/auth': { target: RAISIN_HTTP, changeOrigin: true },
  '/api': { target: RAISIN_HTTP, changeOrigin: true },
};

export default defineConfig({
  plugins: [reactRouter()],
  server: {
    port: 5177,
    strictPort: true,
    proxy,
  },
  preview: {
    port: 5177,
    strictPort: true,
    proxy,
  },
});
