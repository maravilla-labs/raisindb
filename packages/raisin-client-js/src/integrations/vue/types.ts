import type { IdentityUser } from '../../auth';
import type { ConnectionState } from '../../connection';
import type { SubscriptionFilters, EventMessage } from '../../protocol';
import type { RaisinClient } from '../../client';
import type { Database } from '../../database';
import type { ChatMessage, ConversationListItem } from '../../types/chat';
import type {
  ConversationStore,
  ConversationStoreSnapshot,
  ToolCallInfo,
} from '../../stores/conversation-store';
import type { ConversationListStore } from '../../stores/conversation-list-store';
import type { PlanProjection } from '../../utils/plan-projection';

/** Minimal mutable ref shape (structural subset of Vue's `Ref<T>`) */
export interface VueRef<T> {
  value: T;
}

/**
 * Minimal read-only ref shape (structural subset of Vue's `ComputedRef<T>`).
 *
 * At runtime these ARE real `vue.computed()` refs (created via the module you
 * pass to `createRaisinVue`), but the declared type is structural — it lacks
 * Vue's branded `ComputedRefSymbol`, so TypeScript will not accept it as a
 * `watch()` / `watchEffect()` source directly. Wrap it in a getter instead:
 *
 * ```ts
 * const { data } = useSql(db, sql);
 * watch(() => data.value, (rows) => { ... });   // ✅ typechecks
 * watch(data, (rows) => { ... });               // ❌ TS2345 (works at runtime)
 * ```
 */
export interface VueComputedRef<T> {
  readonly value: T;
}

/**
 * Minimal subset of the Vue 3 API needed by the composables.
 *
 * The SDK follows the same "bring your own framework" pattern as the React
 * integration (`createRaisinReact(React)`): instead of importing from `vue`
 * (which would make Vue a dependency of the SDK), you pass the Vue module —
 * or any object with compatible `ref` / `computed` / `onUnmounted` — to
 * `createRaisinVue()`.
 *
 * The signatures are intentionally loose (`any`-based `ref`) so the real
 * `import * as vue from 'vue'` module is structurally assignable despite
 * Vue's `UnwrapRef` generics. Composables re-narrow the types internally.
 */
export interface VueLike {
  /** Vue's `ref()` (or `shallowRef()`); snapshots are replaced wholesale. */
  ref(value: any): { value: any };
  /** Vue's `computed()` */
  computed<T>(getter: () => T): VueComputedRef<T>;
  /** Vue's `onUnmounted()` lifecycle hook */
  onUnmounted(fn: () => void): void;
}

/** Return type for useAuth() */
export interface UseAuthReturn {
  user: VueComputedRef<IdentityUser | null>;
  isAuthenticated: VueComputedRef<boolean>;
  isLoading: VueComputedRef<boolean>;
  login: (email: string, password: string, repository: string) => Promise<IdentityUser>;
  register: (email: string, password: string, repository: string, displayName?: string) => Promise<IdentityUser>;
  logout: (options?: { disconnect?: boolean; reconnect?: boolean }) => Promise<void>;
  initSession: (repository: string) => Promise<IdentityUser | null>;
}

/** Return type for useConnection() */
export interface UseConnectionReturn {
  state: VueComputedRef<ConnectionState>;
  isConnected: VueComputedRef<boolean>;
  isReady: VueComputedRef<boolean>;
  connect: () => Promise<void>;
  disconnect: () => void;
}

/** Return type for useSql<T>() */
export interface UseSqlReturn<T> {
  data: VueComputedRef<T[] | null>;
  isLoading: VueComputedRef<boolean>;
  error: VueComputedRef<Error | null>;
  refetch: () => Promise<void>;
}

/** Handle returned by useSubscription() */
export interface UseSubscriptionReturn {
  /** Stop receiving events (also called automatically on unmount) */
  unsubscribe: () => void;
}

/** Return type for useConversation() */
export interface UseConversationReturn {
  conversation: VueComputedRef<ConversationStoreSnapshot['conversation']>;
  messages: VueComputedRef<ChatMessage[]>;
  isStreaming: VueComputedRef<boolean>;
  isWaiting: VueComputedRef<boolean>;
  streamingText: VueComputedRef<string>;
  error: VueComputedRef<string | null>;
  activeToolCalls: VueComputedRef<ToolCallInfo[]>;
  plans: VueComputedRef<PlanProjection[]>;
  isLoading: VueComputedRef<boolean>;
  conversationPath: VueComputedRef<string | null>;
  sendMessage: (content: string) => Promise<void>;
  approvePlan: (planPath: string) => Promise<void>;
  rejectPlan: (planPath: string, feedback?: string) => Promise<void>;
  stop: () => void;
  loadMessages: () => Promise<ChatMessage[]>;
  /** The underlying store, for advanced use */
  store: ConversationStore;
}

/** Return type for useConversationList() */
export interface UseConversationListReturn {
  conversations: VueComputedRef<ConversationListItem[]>;
  totalUnreadCount: VueComputedRef<number>;
  isLoading: VueComputedRef<boolean>;
  error: VueComputedRef<string | null>;
  createConversation: (options: { participant: string; subject?: string; input?: Record<string, unknown> }) => Promise<ConversationListItem>;
  deleteConversation: (conversationPath: string) => Promise<void>;
  markAsRead: (conversationPath: string) => Promise<void>;
  /** Reload the list from the server */
  refresh: () => Promise<void>;
  /** The underlying store, for advanced use */
  store: ConversationListStore;
}

export type { IdentityUser, ConnectionState, SubscriptionFilters, EventMessage, RaisinClient, Database };
