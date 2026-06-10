import adapter from '@sveltejs/adapter-node';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
  // vitePreprocess lets us use <script lang="ts"> in .svelte components
  // (TypeScript is stripped by esbuild, no separate typecheck step).
  preprocess: vitePreprocess(),
  kit: {
    // Real SSR: the production build is a Node server (`node build`),
    // not a static SPA. `npm run preview` serves it on port 5175.
    adapter: adapter(),
  },
};
