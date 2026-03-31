import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';
import basicSsl from '@vitejs/plugin-basic-ssl';

export default defineConfig({
  plugins: [sveltekit(), basicSsl()],
  server: {
    port: 5173,
    host: true,  // Expose on all network interfaces for Quest browser access
    https: true, // Enable HTTPS for WebXR support
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
