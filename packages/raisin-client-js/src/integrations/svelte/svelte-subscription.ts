import type { Database } from '../../database';
import type { EventMessage, SubscriptionFilters } from '../../protocol';

export interface SubscriptionAdapter {
  destroy: () => void;
}

/**
 * Create an event subscription adapter for Svelte 5.
 *
 * Pure side-effect adapter — subscribes to database events and invokes the
 * callback on each matching event. Call `destroy()` to unsubscribe.
 *
 * @example
 * ```typescript
 * // In a .svelte component's script block or .svelte.ts file
 * import { createSubscriptionAdapter } from '@raisindb/client/svelte';
 * import { db } from '$lib/raisin';
 *
 * const sub = createSubscriptionAdapter(
 *   db,
 *   { workspace: 'content', node_type: 'Post' },
 *   (event) => { console.log('Post changed:', event); },
 * );
 *
 * // In onDestroy or $effect cleanup:
 * sub.destroy();
 * ```
 */
export function createSubscriptionAdapter(
  database: Database,
  filters: SubscriptionFilters,
  callback: (event: EventMessage) => void,
): SubscriptionAdapter {
  let unsubscribe: (() => Promise<void>) | undefined;
  let cancelled = false;

  database.events().subscribe(filters, (event) => {
    if (!cancelled) callback(event);
  }).then((sub) => {
    if (cancelled) {
      sub.unsubscribe();
    } else {
      unsubscribe = () => sub.unsubscribe();
    }
  });

  return {
    destroy() {
      cancelled = true;
      unsubscribe?.();
    },
  };
}
