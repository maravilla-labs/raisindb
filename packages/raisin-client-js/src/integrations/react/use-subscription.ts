import type { ReactLikeWithContext, ReactContext, RaisinContextValue, UseSubscriptionOptions } from './types';
import type { EventMessage } from '../../protocol';

export function createUseSubscription(
  react: ReactLikeWithContext,
  context: ReactContext<RaisinContextValue | null>,
): (options: UseSubscriptionOptions, callback: (event: EventMessage) => void) => void {
  return function useSubscription(
    options: UseSubscriptionOptions,
    callback: (event: EventMessage) => void,
  ): void {
    const { useEffect, useRef, useContext } = react;

    const ctx = useContext(context);
    if (!ctx) {
      throw new Error('useSubscription must be used within a <RaisinProvider>');
    }
    const { client, repository } = ctx;

    // callbackRef pattern: avoid re-subscribing when callback identity changes
    const callbackRef = useRef(callback);
    callbackRef.current = callback;

    const enabled = options.enabled !== false;

    useEffect(() => {
      if (!enabled || !repository || !options.workspace) return;

      const db = client.database(repository);
      let unsubscribe: (() => Promise<void>) | undefined;
      let cancelled = false;

      db.events().subscribe(
        {
          workspace: options.workspace,
          event_types: options.event_types,
          path: options.path,
          node_type: options.node_type,
          include_node: options.include_node,
        },
        (event) => {
          if (!cancelled) {
            callbackRef.current(event);
          }
        },
      ).then((sub) => {
        if (cancelled) {
          sub.unsubscribe();
        } else {
          unsubscribe = () => sub.unsubscribe();
        }
      });

      return () => {
        cancelled = true;
        unsubscribe?.();
      };
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [client, repository, enabled, options.workspace, options.path, options.node_type, options.include_node]);
  };
}
