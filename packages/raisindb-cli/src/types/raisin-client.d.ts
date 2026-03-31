/**
 * Type declarations for @raisindb/client
 * This provides types for the dynamic import of the client
 */

declare module '@raisindb/client' {
  import { EventEmitter } from 'events';

  export type LogLevel = 'debug' | 'info' | 'warn' | 'error' | 'none';

  export interface ClientOptions {
    connection?: {
      autoReconnect?: boolean;
      reconnectInterval?: number;
      maxReconnectAttempts?: number;
    };
    tokenStorage?: {
      getToken(): string | null;
      setToken(token: string): void;
      clearToken(): void;
    };
    requestTimeout?: number;
    tenantId?: string;
    defaultBranch?: string;
    mode?: 'websocket' | 'http' | 'hybrid';
    logLevel?: LogLevel;
  }

  export interface Credentials {
    username?: string;
    password?: string;
    accessToken?: string;
  }

  export interface SubscriptionFilters {
    workspace?: string;
    path?: string;
    node_type?: string;
    event_types?: string[];
  }

  export interface EventMessage {
    event_type: string;
    subscription_id: string;
    payload: unknown;
    timestamp: number;
  }

  export interface Subscription {
    id: string;
    unsubscribe(): Promise<void>;
    isActive(): boolean;
  }

  export type EventCallback = (event: EventMessage) => void;

  export interface EventSubscriptions {
    subscribe(filters: Partial<SubscriptionFilters>, callback: EventCallback): Promise<Subscription>;
    subscribeToTypes(eventTypes: string[], callback: EventCallback): Promise<Subscription>;
    subscribeToPath(path: string, callback: EventCallback): Promise<Subscription>;
    subscribeToNodeType(nodeType: string, callback: EventCallback): Promise<Subscription>;
  }

  export interface Database {
    events(): EventSubscriptions;
    executeSql(query: string): Promise<unknown>;
    getNode(workspace: string, path: string): Promise<unknown>;
    listNodes(workspace: string, path: string): Promise<unknown[]>;
  }

  export class RaisinClient extends EventEmitter {
    constructor(url: string, options?: ClientOptions);
    connect(): Promise<void>;
    disconnect(): void;
    authenticate(credentials: Credentials): Promise<void>;
    database(name: string): Database;
    isConnected(): boolean;
    isAuthenticated(): boolean;
    getTenantId(): string;
    setBranch(branch: string): void;
    getBranch(): string;
  }
}
