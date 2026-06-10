import type {
  VueLike,
  VueRef,
  UseConversationReturn,
  UseConversationListReturn,
} from './types';
import {
  ConversationStore,
  type ConversationStoreOptions,
  type ConversationStoreSnapshot,
} from '../../stores/conversation-store';
import {
  ConversationListStore,
  type ConversationListStoreOptions,
  type ConversationListSnapshot,
} from '../../stores/conversation-list-store';
import { logger } from '../../logger';

export function createUseConversation(
  vue: VueLike,
): (options: ConversationStoreOptions) => UseConversationReturn {
  return function useConversation(options: ConversationStoreOptions): UseConversationReturn {
    const store = new ConversationStore(options);
    const snapshot = vue.ref(store.getSnapshot()) as VueRef<ConversationStoreSnapshot>;
    const unsubscribe = store.subscribe((s) => { snapshot.value = s; });

    vue.onUnmounted(() => {
      unsubscribe();
      store.destroy();
    });

    return {
      conversation: vue.computed(() => snapshot.value.conversation),
      messages: vue.computed(() => snapshot.value.messages),
      isStreaming: vue.computed(() => snapshot.value.isStreaming),
      isWaiting: vue.computed(() => snapshot.value.isWaiting),
      streamingText: vue.computed(() => snapshot.value.streamingText),
      error: vue.computed(() => snapshot.value.error),
      activeToolCalls: vue.computed(() => snapshot.value.activeToolCalls),
      plans: vue.computed(() => snapshot.value.plans),
      isLoading: vue.computed(() => snapshot.value.isLoading),
      conversationPath: vue.computed(() => snapshot.value.conversationPath),
      sendMessage: (content: string) => store.sendMessage(content),
      approvePlan: async (planPath: string) => { await store.approvePlan(planPath); },
      rejectPlan: async (planPath: string, feedback?: string) => { await store.rejectPlan(planPath, feedback); },
      stop: () => store.stop(),
      loadMessages: () => store.loadMessages(),
      store,
    };
  };
}

export function createUseConversationList(
  vue: VueLike,
): (options: ConversationListStoreOptions) => UseConversationListReturn {
  return function useConversationList(options: ConversationListStoreOptions): UseConversationListReturn {
    const store = new ConversationListStore(options);
    const snapshot = vue.ref(store.getSnapshot()) as VueRef<ConversationListSnapshot>;
    const unsubscribe = store.subscribe((s) => { snapshot.value = s; });

    // Initial load (same behavior as the React useConversationList hook).
    store.load().catch((err) => {
      logger.error('[useConversationList] Initial load failed:', err);
    });

    vue.onUnmounted(() => {
      unsubscribe();
      store.destroy();
    });

    return {
      conversations: vue.computed(() => snapshot.value.conversations),
      totalUnreadCount: vue.computed(() => snapshot.value.totalUnreadCount),
      isLoading: vue.computed(() => snapshot.value.isLoading),
      error: vue.computed(() => snapshot.value.error),
      createConversation: (opts) => store.createConversation(opts),
      deleteConversation: (conversationPath: string) => store.deleteConversation(conversationPath),
      markAsRead: (conversationPath: string) => store.markAsRead(conversationPath),
      refresh: () => store.load(),
      store,
    };
  };
}
