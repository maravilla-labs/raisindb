import { defineConfig } from 'tsup';

export default defineConfig({
  entry: {
    index: 'src/index.ts',
    // Deep-import entry points (see package.json "exports")
    react: 'src/integrations/react/index.ts',
    svelte: 'src/integrations/svelte/index.ts',
    vue: 'src/integrations/vue/index.ts',
  },
  format: ['cjs', 'esm'],
  dts: true,
  sourcemap: true,
  clean: true,
});
