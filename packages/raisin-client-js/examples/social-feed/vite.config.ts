import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    port: Number(process.env.VITE_PORT) || 3000,
    open: process.env.VITE_NO_OPEN !== 'true',
  },
  resolve: {
    alias: {
      '@': '/src',
      // Polyfill Node.js modules for browser
      events: 'events',
    },
  },
  optimizeDeps: {
    esbuildOptions: {
      // Node.js global to browser globalThis
      define: {
        global: 'globalThis',
      },
    },
  },
});
