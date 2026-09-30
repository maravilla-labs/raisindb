import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    // Temp HOME, no RAISINDB_* env: tests never see a real ~/.raisinrc.
    setupFiles: ['./vitest.setup.ts'],
  },
});
