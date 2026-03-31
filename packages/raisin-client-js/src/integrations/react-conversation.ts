/**
 * React adapters for the RaisinDB conversation stores.
 *
 * Since the SDK does not depend on React, the hooks accept React as a
 * parameter to avoid a hard dependency while providing full type safety.
 *
 * @example
 * ```tsx
 * import React from 'react';
 * import { useConversation, useConversationList } from '@raisindb/client';
 *
 * function Chat() {
 *   const chat = useConversation(React, {
 *     database: db,
 *     conversationPath: '/...',
 *   });
 *   return (
 *     <div>
 *       {chat.messages.map((m, i) => <div key={i}>{m.content}</div>)}
 *       {chat.isStreaming && <p>{chat.streamingText}</p>}
 *     </div>
 *   );
 * }
 * ```
 */

import {
  ConversationStore,
  type ConversationStoreOptions,
  type ConversationStoreSnapshot,
} from '../stores/conversation-store';
import {
  ConversationListStore,
  type ConversationListStoreOptions,
  type ConversationListSnapshot,
} from '../stores/conversation-list-store';
import type { ChatMessage, ConversationListItem } from '../types/chat';

/** Minimal subset of the React API needed by the hooks */
export interface ReactLike {
  useState<T>(initial: T | (() => T)): [T, (value: T | ((prev: T) => T)) => void];
  useEffect(effect: () => void | (() => void), deps?: unknown[]): void;
  useRef<T>(initial: T): { current: T };
  useCallback<T extends (...args: never[]) => unknown>(fn: T, deps: unknown[]): T;
}

// ---------------------------------------------------------------------------
// useConversation
// ---------------------------------------------------------------------------

const INITIAL_SNAPSHOT: ConversationStoreSnapshot = {
  conversation: null,
  messages: [],
  isStreaming: false,
  isWaiting: false,
  streamingText: '',
  error: null,
  activeToolCalls: [],
  plans: [],
  isLoading: false,
  conversationPath: null,
};

export interface UseConversationReturn extends ConversationStoreSnapshot {
  sendMessage: (content: string) => Promise<void>;
  approvePlan: (planPath: string) => Promise<void>;
  rejectPlan: (planPath: string, feedback?: string) => Promise<void>;
  stop: () => void;
  loadMessages: () => Promise<ChatMessage[]>;
}

export function useConversation(
  react: ReactLike,
  options: ConversationStoreOptions,
): UseConversationReturn {
  const { useState, useEffect, useRef, useCallback } = react;

  const [snapshot, setSnapshot] = useState<ConversationStoreSnapshot>(INITIAL_SNAPSHOT);
  const storeRef = useRef<ConversationStore | null>(null);

  useEffect(() => {
    const store = new ConversationStore(options);
    storeRef.current = store;

    const unsubscribe = store.subscribe((next) => setSnapshot(next));

    return () => {
      unsubscribe();
      store.destroy();
      storeRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.conversationPath]);

  const sendMessage = useCallback(async (content: string) => {
    await storeRef.current?.sendMessage(content);
  }, []);

  const approvePlan = useCallback(async (planPath: string) => {
    await storeRef.current?.approvePlan(planPath);
  }, []);

  const rejectPlan = useCallback(async (planPath: string, feedback?: string) => {
    await storeRef.current?.rejectPlan(planPath, feedback);
  }, []);

  const stop = useCallback(() => { storeRef.current?.stop(); }, []);

  const loadMessages = useCallback(async () => {
    return (await storeRef.current?.loadMessages()) ?? [];
  }, []);

  return {
    ...snapshot,
    sendMessage,
    approvePlan,
    rejectPlan,
    stop,
    loadMessages,
  };
}

// ---------------------------------------------------------------------------
// useConversationList
// ---------------------------------------------------------------------------

const INITIAL_LIST_SNAPSHOT: ConversationListSnapshot = {
  conversations: [],
  totalUnreadCount: 0,
  isLoading: false,
  error: null,
};

export interface UseConversationListReturn extends ConversationListSnapshot {
  createConversation: (options: { participant: string; subject?: string }) => Promise<ConversationListItem>;
  deleteConversation: (conversationPath: string) => Promise<void>;
  markAsRead: (conversationPath: string) => Promise<void>;
}

export function useConversationList(
  react: ReactLike,
  options: ConversationListStoreOptions,
): UseConversationListReturn {
  const { useState, useEffect, useRef, useCallback } = react;

  const [snapshot, setSnapshot] = useState<ConversationListSnapshot>(INITIAL_LIST_SNAPSHOT);
  const storeRef = useRef<ConversationListStore | null>(null);

  useEffect(() => {
    const store = new ConversationListStore(options);
    storeRef.current = store;

    const unsubscribe = store.subscribe((next) => setSnapshot(next));
    store.load();

    return () => {
      unsubscribe();
      store.destroy();
      storeRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [options.type, options.realtime]);

  const createConversation = useCallback(async (opts: { participant: string; subject?: string }) => {
    return storeRef.current!.createConversation(opts);
  }, []);

  const deleteConversation = useCallback(async (conversationPath: string) => {
    await storeRef.current?.deleteConversation(conversationPath);
  }, []);

  const markAsRead = useCallback(async (conversationPath: string) => {
    await storeRef.current?.markAsRead(conversationPath);
  }, []);

  return {
    ...snapshot,
    createConversation,
    deleteConversation,
    markAsRead,
  };
}
