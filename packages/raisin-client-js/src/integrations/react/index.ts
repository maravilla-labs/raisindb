/**
 * React integration layer for RaisinDB.
 *
 * Provides a `createRaisinReact(React)` factory that returns a Provider
 * component and hooks for auth, connection, queries, and subscriptions.
 *
 * Uses the "bring your own React" pattern to avoid a peer dependency.
 *
 * @example
 * ```ts
 * import React from 'react';
 * import { RaisinClient, LocalStorageTokenStorage, createRaisinReact } from '@raisindb/client';
 *
 * const client = new RaisinClient('wss://localhost:8443/sys/default/myrepo', {
 *   tokenStorage: new LocalStorageTokenStorage('myrepo'),
 * });
 *
 * export const {
 *   RaisinProvider, useAuth, useConnection, useDatabase,
 *   useSql, useSubscription, useConversation, useFlow,
 * } = createRaisinReact(React);
 * ```
 */

import type {
  ReactLikeWithContext,
  RaisinProviderProps,
  RaisinContextValue,
  UseAuthReturn,
  UseConnectionReturn,
  UseSqlOptions,
  UseSqlReturn,
  UseSubscriptionOptions,
} from './types';

import type { RaisinClient } from '../../client';
import type { Database } from '../../database';
import type { EventMessage } from '../../protocol';
import type {
  ConversationStoreOptions,
} from '../../stores/conversation-store';
import type { ConversationListStoreOptions } from '../../stores/conversation-list-store';

import { createUseAuth } from './use-auth';
import { createUseConnection } from './use-connection';
import { createUseSql } from './use-sql';
import { createUseSubscription } from './use-subscription';

import {
  useConversation as rawUseConversation,
  useConversationList as rawUseConversationList,
  type UseConversationReturn,
  type UseConversationListReturn,
} from '../react-conversation';
import {
  useFlow as rawUseFlow,
  type UseFlowOptions,
  type UseFlowReturn,
} from '../react-flow';

export interface RaisinReact {
  RaisinProvider: (props: RaisinProviderProps) => any;
  useRaisinClient: () => RaisinClient;
  useDatabase: (repository?: string) => Database;
  useAuth: () => UseAuthReturn;
  useConnection: () => UseConnectionReturn;
  useSql: <T = Record<string, unknown>>(sql: string, params?: unknown[], options?: UseSqlOptions) => UseSqlReturn<T>;
  useSubscription: (options: UseSubscriptionOptions, callback: (event: EventMessage) => void) => void;
  useConversation: (options: ConversationStoreOptions) => UseConversationReturn;
  useConversationList: (options: ConversationListStoreOptions) => UseConversationListReturn;
  useFlow: (options: UseFlowOptions) => UseFlowReturn;
}

/**
 * Create the full React integration for RaisinDB.
 *
 * Accepts a React instance and returns a Provider component plus hooks.
 * The React instance must include `createContext`, `useContext`, `useMemo`,
 * and `createElement` in addition to the base `ReactLike` interface.
 *
 * @param react - The React instance
 * @returns Provider component and hooks
 */
export function createRaisinReact(react: ReactLikeWithContext): RaisinReact {
  const RaisinContext = react.createContext<RaisinContextValue | null>(null);

  // --- Provider ---
  function RaisinProvider(props: RaisinProviderProps): any {
    const { client, repository, children } = props;

    const value = react.useMemo(
      () => ({ client, repository }),
      [client, repository],
    );

    return react.createElement(
      RaisinContext.Provider as unknown,
      { value },
      children,
    );
  }

  // --- Context accessors ---
  function useRaisinClient(): RaisinClient {
    const ctx = react.useContext(RaisinContext);
    if (!ctx) {
      throw new Error('useRaisinClient must be used within a <RaisinProvider>');
    }
    return ctx.client;
  }

  function useDatabase(repository?: string): Database {
    const ctx = react.useContext(RaisinContext);
    if (!ctx) {
      throw new Error('useDatabase must be used within a <RaisinProvider>');
    }
    const repo = repository ?? ctx.repository;
    if (!repo) {
      throw new Error('useDatabase requires a repository — pass it as an argument or set it on <RaisinProvider>');
    }
    return react.useMemo(() => ctx.client.database(repo), [ctx.client, repo]);
  }

  // --- Core hooks ---
  const useAuth = createUseAuth(react, RaisinContext);
  const useConnection = createUseConnection(react, RaisinContext);
  const useSql = createUseSql(react, RaisinContext);
  const useSubscription = createUseSubscription(react, RaisinContext);

  // --- Pre-bound existing hooks ---
  function useConversation(options: ConversationStoreOptions): UseConversationReturn {
    return rawUseConversation(react, options);
  }

  function useConversationList(options: ConversationListStoreOptions): UseConversationListReturn {
    return rawUseConversationList(react, options);
  }

  function useFlow(options: UseFlowOptions): UseFlowReturn {
    return rawUseFlow(react, options);
  }

  return {
    RaisinProvider,
    useRaisinClient,
    useDatabase,
    useAuth,
    useConnection,
    useSql,
    useSubscription,
    useConversation,
    useConversationList,
    useFlow,
  };
}

// Re-export types
export type {
  ReactLikeWithContext,
  RaisinProviderProps,
  RaisinContextValue,
  UseAuthReturn,
  UseConnectionReturn,
  UseSqlOptions,
  UseSqlReturn,
  UseSubscriptionOptions,
} from './types';
