import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit()],
  server: {
    port: 5173
  },
  optimizeDeps: {
    // Exclude transformers.js from pre-bundling as it loads WASM/ONNX dynamically
    exclude: ['@xenova/transformers']
  },
  build: {
    // Increase chunk size warning limit for WASM files
    chunkSizeWarningLimit: 5000
  }
});
