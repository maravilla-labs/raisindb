import type { Config } from '@react-router/dev/config';

export default {
  // Enable server-side rendering
  ssr: true,

  // App directory (where routes, components, etc. live)
  appDirectory: 'app',

  // Build output directory
  buildDirectory: 'build',

  // Server build output
  serverBuildFile: 'index.js',

  // Server module format
  serverModuleFormat: 'esm',

  // Public path for assets
  publicPath: '/assets/',

  // Prerender routes for static generation (optional)
  // async prerender() {
  //   return ['/'];
  // },
} satisfies Config;
