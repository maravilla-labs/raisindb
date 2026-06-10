/**
 * Vue 3 integration layer for RaisinDB.
 *
 * Provides a `createRaisinVue(vue)` factory that returns composables for
 * auth, connection, SQL queries, node-event subscriptions, and the
 * conversation stores.
 *
 * Uses the same "bring your own framework" pattern as the React integration
 * (`createRaisinReact(React)`): the SDK never imports `vue`, so Vue stays
 * out of the SDK's dependency graph and the main `.` bundle. You pass the
 * Vue module (its `ref`, `computed`, and `onUnmounted` are all that is
 * needed) once at setup time.
 *
 * Each composable creates its underlying store/adapter when called and
 * destroys it automatically in `onUnmounted`, so call them from
 * `setup()` / `<script setup>`.
 *
 * @example
 * ```ts
 * // lib/raisin-vue.ts
 * import * as vue from 'vue';
 * import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';
 * import { createRaisinVue } from '@raisindb/client/vue';
 *
 * export const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo', {
 *   tokenStorage: new LocalStorageTokenStorage('myapp'),
 * });
 * export const db = client.database('myrepo');
 *
 * export const {
 *   useAuth, useConnection, useSql, useSubscription,
 *   useConversation, useConversationList,
 * } = createRaisinVue(vue);
 * ```
 *
 * @example In a component:
 * ```vue
 * <script setup lang="ts">
 * import { client, db, useConversation } from '@/lib/raisin-vue';
 *
 * const chat = useConversation({
 *   database: db,
 *   createOptions: { participant: '/agents/support' },
 * });
 * </script>
 *
 * <template>
 *   <div v-for="(m, i) in chat.messages.value" :key="i">{{ m.content }}</div>
 *   <p v-if="chat.isStreaming.value">{{ chat.streamingText.value }}</p>
 * </template>
 * ```
 */

import type {
  VueLike,
  UseAuthReturn,
  UseConnectionReturn,
  UseSqlReturn,
  UseSubscriptionReturn,
  UseConversationReturn,
  UseConversationListReturn,
} from './types';
import type { RaisinClient } from '../../client';
import type { Database } from '../../database';
import type { EventMessage, SubscriptionFilters } from '../../protocol';
import type { ConversationStoreOptions } from '../../stores/conversation-store';
import type { ConversationListStoreOptions } from '../../stores/conversation-list-store';
import type { SqlAdapterOptions } from '../svelte/svelte-sql';

import { createUseAuth } from './vue-auth';
import { createUseConnection } from './vue-connection';
import { createUseSql } from './vue-sql';
import { createUseSubscription } from './vue-subscription';
import { createUseConversation, createUseConversationList } from './vue-conversation';

export interface RaisinVue {
  /** Reactive auth state + login/register/logout/initSession actions */
  useAuth: (client: RaisinClient) => UseAuthReturn;
  /** Reactive connection state (state / isConnected / isReady) */
  useConnection: (client: RaisinClient) => UseConnectionReturn;
  /** Run a SQL query with reactive data/isLoading/error + optional realtime refetch */
  useSql: <T = Record<string, unknown>>(
    database: Database,
    sql: string,
    params?: unknown[],
    options?: SqlAdapterOptions,
  ) => UseSqlReturn<T>;
  /** Subscribe to node events; unsubscribes automatically on unmount */
  useSubscription: (
    database: Database,
    filters: SubscriptionFilters,
    callback: (event: EventMessage) => void,
  ) => UseSubscriptionReturn;
  /** Bind a ConversationStore (chat: messages, streaming, tool calls, plans) */
  useConversation: (options: ConversationStoreOptions) => UseConversationReturn;
  /** Bind a ConversationListStore (inbox-style conversation list) */
  useConversationList: (options: ConversationListStoreOptions) => UseConversationListReturn;
}

/**
 * Create the full Vue 3 integration for RaisinDB.
 *
 * @param vue - The Vue module (or any object providing `ref`, `computed`, `onUnmounted`)
 * @returns The set of composables
 */
export function createRaisinVue(vue: VueLike): RaisinVue {
  return {
    useAuth: createUseAuth(vue),
    useConnection: createUseConnection(vue),
    useSql: createUseSql(vue),
    useSubscription: createUseSubscription(vue),
    useConversation: createUseConversation(vue),
    useConversationList: createUseConversationList(vue),
  };
}

// Re-export types
export type {
  VueLike,
  VueRef,
  VueComputedRef,
  UseAuthReturn,
  UseConnectionReturn,
  UseSqlReturn,
  UseSubscriptionReturn,
  UseConversationReturn,
  UseConversationListReturn,
} from './types';
export type { SqlAdapterOptions as UseSqlOptions } from '../svelte/svelte-sql';
