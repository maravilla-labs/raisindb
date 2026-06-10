import type { VueLike, VueRef, UseSqlReturn } from './types';
import type { Database } from '../../database';
// Framework-free snapshot adapter shared with the Svelte integration.
import {
  createSqlAdapter,
  type SqlAdapterOptions,
  type SqlSnapshot,
} from '../svelte/svelte-sql';

export function createUseSql(
  vue: VueLike,
): <T = Record<string, unknown>>(
  database: Database,
  sql: string,
  params?: unknown[],
  options?: SqlAdapterOptions,
) => UseSqlReturn<T> {
  return function useSql<T = Record<string, unknown>>(
    database: Database,
    sql: string,
    params?: unknown[],
    options?: SqlAdapterOptions,
  ): UseSqlReturn<T> {
    const adapter = createSqlAdapter<T>(database, sql, params, options);
    const snapshot = vue.ref(adapter.getSnapshot()) as VueRef<SqlSnapshot<T>>;
    const unsubscribe = adapter.subscribe((s) => { snapshot.value = s; });

    vue.onUnmounted(() => {
      unsubscribe();
      adapter.destroy();
    });

    return {
      data: vue.computed(() => snapshot.value.data),
      isLoading: vue.computed(() => snapshot.value.isLoading),
      error: vue.computed(() => snapshot.value.error),
      refetch: adapter.refetch,
    };
  };
}
