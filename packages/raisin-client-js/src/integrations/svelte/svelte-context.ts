import type { RaisinClient } from '../../client';

/**
 * Context value for the RaisinDB Svelte integration.
 *
 * Use with Svelte's `setContext`/`getContext` in `.svelte` files.
 *
 * @example Setup in `+layout.svelte`:
 * ```svelte
 * <script>
 *   import { setContext } from 'svelte';
 *   import { RAISIN_CONTEXT_KEY, type RaisinContext } from '@raisindb/client/svelte';
 *   import { client } from '$lib/raisin';
 *
 *   setContext<RaisinContext>(RAISIN_CONTEXT_KEY, { client, repository: 'myrepo' });
 * </script>
 *
 * {@render children()}
 * ```
 *
 * @example Usage in a child component:
 * ```svelte
 * <script>
 *   import { getContext } from 'svelte';
 *   import { RAISIN_CONTEXT_KEY, type RaisinContext } from '@raisindb/client/svelte';
 *
 *   const { client, repository } = getContext<RaisinContext>(RAISIN_CONTEXT_KEY);
 *   const db = client.database(repository!);
 * </script>
 * ```
 */
export interface RaisinContext {
  client: RaisinClient;
  repository?: string;
}

/**
 * Shared context key for RaisinDB. Use this with `setContext` and `getContext`
 * to ensure consistent key usage across your application.
 */
export const RAISIN_CONTEXT_KEY: unique symbol = Symbol('raisin');
