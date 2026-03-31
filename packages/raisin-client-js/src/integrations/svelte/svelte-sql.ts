import type { Database } from '../../database';
import type { SqlResult } from '../../protocol';

export interface SqlAdapterOptions {
  enabled?: boolean;
  refetchOnReconnect?: boolean;
  realtime?: {
    workspace: string;
    eventTypes?: string[];
    path?: string;
    nodeType?: string;
  };
}

export interface SqlSnapshot<T> {
  data: T[] | null;
  isLoading: boolean;
  error: Error | null;
}

export interface SqlAdapter<T> {
  subscribe: (cb: (snapshot: SqlSnapshot<T>) => void) => () => void;
  getSnapshot: () => SqlSnapshot<T>;
  refetch: () => Promise<void>;
  destroy: () => void;
}

function rowsToObjects<T>(result: SqlResult): T[] {
  return result.rows as T[];
}

/**
 * Create a SQL query adapter for Svelte 5.
 *
 * Executes a SQL query and maintains reactive snapshot state. Supports
 * realtime subscriptions and automatic refetch on reconnect.
 *
 * @example
 * ```typescript
 * // lib/posts.svelte.ts
 * import { createSqlAdapter } from '@raisindb/client/svelte';
 * import { db } from '$lib/raisin';
 *
 * const adapter = createSqlAdapter<{ title: string }>(db, "SELECT * FROM 'content'", [], {
 *   realtime: { workspace: 'content' },
 * });
 * let snapshot = $state(adapter.getSnapshot());
 * adapter.subscribe(s => { snapshot = s; });
 *
 * export const posts = {
 *   get data() { return snapshot.data; },
 *   get isLoading() { return snapshot.isLoading; },
 *   get error() { return snapshot.error; },
 *   refetch: adapter.refetch,
 * };
 * ```
 */
export function createSqlAdapter<T = Record<string, unknown>>(
  database: Database,
  sql: string,
  params?: unknown[],
  options?: SqlAdapterOptions,
): SqlAdapter<T> {
  let snapshot: SqlSnapshot<T> = {
    data: null,
    isLoading: false,
    error: null,
  };

  const listeners = new Set<(s: SqlSnapshot<T>) => void>();
  let destroyed = false;

  function emit() {
    for (const cb of listeners) cb(snapshot);
  }

  function update(partial: Partial<SqlSnapshot<T>>) {
    snapshot = { ...snapshot, ...partial };
    emit();
  }

  async function fetchData() {
    if (destroyed) return;
    update({ isLoading: true, error: null });
    try {
      const result = await database.executeSql(sql, params);
      if (!destroyed) {
        update({ data: rowsToObjects<T>(result), isLoading: false });
      }
    } catch (err) {
      if (!destroyed) {
        update({
          error: err instanceof Error ? err : new Error(String(err)),
          isLoading: false,
        });
      }
    }
  }

  // Cleanup handles
  const cleanups: (() => void)[] = [];

  // Initial fetch
  const enabled = options?.enabled !== false;
  if (enabled) {
    fetchData();
  }

  // Refetch on reconnect
  const refetchOnReconnect = options?.refetchOnReconnect !== false;
  if (enabled && refetchOnReconnect) {
    const client = (database as any)._client;
    if (client?.onReconnected) {
      const unsub = client.onReconnected(() => { fetchData(); });
      cleanups.push(unsub);
    }
  }

  // Realtime subscription
  const realtime = options?.realtime;
  if (enabled && realtime) {
    let unsubscribe: (() => Promise<void>) | undefined;
    let cancelled = false;

    database.events().subscribe(
      {
        workspace: realtime.workspace,
        event_types: realtime.eventTypes,
        path: realtime.path,
        node_type: realtime.nodeType,
      },
      () => { fetchData(); },
    ).then((sub) => {
      if (cancelled) {
        sub.unsubscribe();
      } else {
        unsubscribe = () => sub.unsubscribe();
      }
    });

    cleanups.push(() => {
      cancelled = true;
      unsubscribe?.();
    });
  }

  return {
    subscribe(cb: (s: SqlSnapshot<T>) => void) {
      listeners.add(cb);
      return () => { listeners.delete(cb); };
    },

    getSnapshot: () => snapshot,
    refetch: fetchData,

    destroy() {
      destroyed = true;
      for (const cleanup of cleanups) cleanup();
      listeners.clear();
    },
  };
}
