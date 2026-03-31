import type { ReactLike } from '../react-conversation';
import type { IdentityUser } from '../../auth';
import type { ConnectionState } from '../../connection';
import type { SubscriptionFilters, EventMessage } from '../../protocol';
import type { RaisinClient } from '../../client';
import type { Database } from '../../database';

/**
 * Extended React-like interface that includes context and rendering APIs
 * needed by the Provider and context-consuming hooks.
 */
export interface ReactLikeWithContext extends ReactLike {
  createContext<T>(defaultValue: T): ReactContext<T>;
  useContext<T>(context: ReactContext<T>): T;
  useMemo<T>(factory: () => T, deps: unknown[]): T;
  createElement(type: unknown, props: unknown, ...children: unknown[]): any;
}

/** Minimal React context interface */
export interface ReactContext<T> {
  Provider: unknown;
  Consumer?: unknown;
  displayName?: string;
  /** Default value (used internally by React) */
  _currentValue?: T;
}

/** Props for the RaisinProvider component */
export interface RaisinProviderProps {
  client: RaisinClient;
  repository?: string;
  children?: unknown;
}

/** Value stored in the Raisin context */
export interface RaisinContextValue {
  client: RaisinClient;
  repository?: string;
}

/** Return type for useAuth() */
export interface UseAuthReturn {
  user: IdentityUser | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  login: (email: string, password: string, repository: string) => Promise<IdentityUser>;
  register: (email: string, password: string, repository: string, displayName?: string) => Promise<IdentityUser>;
  logout: (options?: { disconnect?: boolean; reconnect?: boolean }) => Promise<void>;
  initSession: (repository: string) => Promise<IdentityUser | null>;
}

/** Return type for useConnection() */
export interface UseConnectionReturn {
  state: ConnectionState;
  isConnected: boolean;
  isReady: boolean;
  connect: () => Promise<void>;
  disconnect: () => void;
}

/** Options for useSql() */
export interface UseSqlOptions {
  enabled?: boolean;
  repository?: string;
  refetchOnReconnect?: boolean;
  realtime?: {
    workspace: string;
    eventTypes?: string[];
    path?: string;
    nodeType?: string;
  };
}

/** Return type for useSql<T>() */
export interface UseSqlReturn<T> {
  data: T[] | null;
  isLoading: boolean;
  error: Error | null;
  refetch: () => Promise<void>;
}

/** Options for useSubscription() */
export interface UseSubscriptionOptions extends SubscriptionFilters {
  enabled?: boolean;
}

/** Re-export ReactLike for convenience */
export type { ReactLike };
export type { IdentityUser, ConnectionState, SubscriptionFilters, EventMessage, RaisinClient, Database };
