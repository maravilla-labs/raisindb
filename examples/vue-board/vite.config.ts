import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';

// The WebSocket connects straight to the server (not CORS-restricted), but
// the SDK's HTTP calls (login, SQL-over-HTTP fallback, conversations SSE)
// are same-origin via this dev proxy, so the demo needs no CORS entry on
// the server. Alternative: `raisindb cors add http://localhost:5176 --repo
// shiftboard2` and set VITE_RAISIN_HTTP_URL=http://localhost:8081.
const RAISIN_HTTP = process.env.VITE_RAISIN_HTTP_URL ?? 'http://localhost:8081';

const proxy = {
  '/auth': { target: RAISIN_HTTP, changeOrigin: true },
  '/api': { target: RAISIN_HTTP, changeOrigin: true },
};

export default defineConfig({
  plugins: [vue()],
  server: {
    port: 5176,
    strictPort: true,
    proxy,
  },
  preview: {
    port: 5176,
    strictPort: true,
    proxy,
  },
});
