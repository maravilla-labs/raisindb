import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  base: '/admin/',
  plugins: [react(), tailwindcss()],
  build: {
    outDir: '../../crates/raisin-server/.admin-console-dist',
    emptyOutDir: true,
  },
  // Web Worker configuration
  worker: {
    format: 'es',
  },
  // WASM support
  optimizeDeps: {
    // Linked workspace packages must not be dep-cached: Vite's pre-bundle
    // otherwise serves stale code after the linked source changes (e.g. new
    // palette items in the flow designer) until a manual --force restart.
    exclude: ['@raisindb/sql-wasm', '@raisindb/flow-designer'],
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8081',
        changeOrigin: true,
      },
      '/workspaces': {
        target: 'http://localhost:8081',
        changeOrigin: true,
      },
      // Management + SSE event streams (/management/events/jobs, ...)
      '/management': {
        target: 'http://localhost:8081',
        changeOrigin: true,
      },
      // Node-event WebSocket. `ws: true` is required — without it Vite proxies
      // the GET but not the Upgrade, so the handshake fails in dev only and the
      // live feed silently falls back to polling.
      '/ws': {
        target: 'ws://localhost:8081',
        ws: true,
        changeOrigin: true,
      },
    },
  },
})
