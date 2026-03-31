import type { ReactLikeWithContext, ReactContext, RaisinContextValue, UseSqlOptions, UseSqlReturn } from './types';
import type { SqlResult } from '../../protocol';

function rowsToObjects<T>(result: SqlResult): T[] {
  return result.rows as T[];
}

export function createUseSql(
  react: ReactLikeWithContext,
  context: ReactContext<RaisinContextValue | null>,
): <T = Record<string, unknown>>(sql: string, params?: unknown[], options?: UseSqlOptions) => UseSqlReturn<T> {
  return function useSql<T = Record<string, unknown>>(
    sql: string,
    params?: unknown[],
    options?: UseSqlOptions,
  ): UseSqlReturn<T> {
    const { useState, useEffect, useRef, useCallback, useContext } = react;

    const ctx = useContext(context);
    if (!ctx) {
      throw new Error('useSql must be used within a <RaisinProvider>');
    }
    const { client, repository: defaultRepo } = ctx;

    const [data, setData] = useState<T[] | null>(null);
    const [isLoading, setIsLoading] = useState(false);
    const [error, setError] = useState<Error | null>(null);
    const mountedRef = useRef(true);

    const enabled = options?.enabled !== false;
    const repo = options?.repository ?? defaultRepo;
    const refetchOnReconnect = options?.refetchOnReconnect !== false;
    const realtime = options?.realtime;

    // Serialize params for dependency comparison
    const paramsKey = params ? JSON.stringify(params) : '';

    const fetchData = useCallback(async () => {
      if (!repo) return;
      setIsLoading(true);
      setError(null);
      try {
        const db = client.database(repo);
        const result = await db.executeSql(sql, params);
        if (mountedRef.current) {
          setData(rowsToObjects<T>(result));
        }
      } catch (err) {
        if (mountedRef.current) {
          setError(err instanceof Error ? err : new Error(String(err)));
        }
      } finally {
        if (mountedRef.current) {
          setIsLoading(false);
        }
      }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [client, repo, sql, paramsKey]);

    // Initial fetch and re-fetch on query/params change
    useEffect(() => {
      mountedRef.current = true;
      if (enabled) {
        fetchData();
      }
      return () => {
        mountedRef.current = false;
      };
    }, [enabled, fetchData]);

    // Refetch on reconnect
    useEffect(() => {
      if (!enabled || !refetchOnReconnect) return;

      const unsub = client.onReconnected(() => {
        fetchData();
      });
      return unsub;
    }, [client, enabled, refetchOnReconnect, fetchData]);

    // Realtime subscription: refetch on matching events
    useEffect(() => {
      if (!enabled || !realtime || !repo) return;

      const db = client.database(repo);
      let unsubscribe: (() => Promise<void>) | undefined;

      db.events().subscribe(
        {
          workspace: realtime.workspace,
          event_types: realtime.eventTypes,
          path: realtime.path,
          node_type: realtime.nodeType,
        },
        () => {
          fetchData();
        },
      ).then((sub) => {
        unsubscribe = () => sub.unsubscribe();
      });

      return () => {
        unsubscribe?.();
      };
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [client, enabled, repo, realtime?.workspace, realtime?.path, realtime?.nodeType, fetchData]);

    return {
      data,
      isLoading,
      error,
      refetch: fetchData,
    };
  };
}
