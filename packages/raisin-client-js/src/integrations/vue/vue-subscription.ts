import type { VueLike, UseSubscriptionReturn } from './types';
import type { Database } from '../../database';
import type { EventMessage, SubscriptionFilters } from '../../protocol';
// Framework-free side-effect adapter shared with the Svelte integration.
import { createSubscriptionAdapter } from '../svelte/svelte-subscription';

export function createUseSubscription(
  vue: VueLike,
): (
  database: Database,
  filters: SubscriptionFilters,
  callback: (event: EventMessage) => void,
) => UseSubscriptionReturn {
  return function useSubscription(
    database: Database,
    filters: SubscriptionFilters,
    callback: (event: EventMessage) => void,
  ): UseSubscriptionReturn {
    const adapter = createSubscriptionAdapter(database, filters, callback);

    // Automatic cleanup when the component unmounts.
    vue.onUnmounted(() => adapter.destroy());

    return {
      unsubscribe: () => adapter.destroy(),
    };
  };
}
