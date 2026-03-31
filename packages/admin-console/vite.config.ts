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
    exclude: ['@raisindb/sql-wasm'],
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
    },
  },
})
