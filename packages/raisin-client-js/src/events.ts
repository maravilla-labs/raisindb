/**
 * Event subscription system
 */

import {
  EventMessage,
  SubscriptionFilters,
  SubscribePayload,
  UnsubscribePayload,
  SubscriptionResponse,
  RequestType,
} from './protocol';
import { logger } from './logger';

/**
 * Event callback function
 */
export type EventCallback = (event: EventMessage) => void;

/**
 * Subscription interface
 */
export interface Subscription {
  /** Unique subscription ID */
  id: string;
  /** Unsubscribe from events */
  unsubscribe(): Promise<void>;
  /** Check if subscription is active */
  isActive(): boolean;
}

/**
 * Internal tracking for deduplicated subscriptions
 */
interface FilterSubscriptionEntry {
  /** Server-side subscription ID */
  serverId: string;
  /** Original filters (for reconnection) */
  filters: SubscriptionFilters;
  /** All callbacks registered for this filter combination */
  callbacks: Set<EventCallback>;
}

/** Default deduplication window in milliseconds (5 seconds) */
const DEFAULT_DEDUP_WINDOW_MS = 5000;

/**
 * Event handler for managing subscriptions with deduplication
 *
 * Multiple subscribe() calls with identical filters will share a single
 * server-side subscription, reducing network overhead and server load.
 *
 * Event deduplication is enabled by default to prevent duplicate event
 * delivery during rapid reconnection scenarios.
 */
export class EventHandler {
  /** Map: server subscription ID -> filter hash (for event routing) */
  private subscriptions = new Map<string, string>();

  /** Map: filter hash -> subscription entry (for deduplication) */
  private filterSubscriptions = new Map<string, FilterSubscriptionEntry>();

  /** Map: callback -> filter hash (for unsubscribe lookup) */
  private callbackToHash = new Map<EventCallback, string>();

  /** Map: event_id -> expiration timestamp (for event deduplication) */
  private recentEventIds = new Map<string, number>();

  /** Deduplication window in milliseconds */
  private dedupWindowMs: number;

  /** Cleanup timer for expired event IDs */
  private dedupCleanupTimer?: ReturnType<typeof setInterval>;

  private sendRequest: (
    payload: SubscribePayload | UnsubscribePayload,
    requestType: RequestType
  ) => Promise<unknown>;

  constructor(
    sendRequest: (
      payload: SubscribePayload | UnsubscribePayload,
      requestType: RequestType
    ) => Promise<unknown>,
    options?: { dedupWindowMs?: number }
  ) {
    this.sendRequest = sendRequest;
    this.dedupWindowMs = options?.dedupWindowMs ?? DEFAULT_DEDUP_WINDOW_MS;

    // Start cleanup timer for expired event IDs (runs every 10 seconds)
    this.dedupCleanupTimer = setInterval(() => {
      this.cleanupExpiredEventIds();
    }, 10000);
  }

  /**
   * Cleanup resources (call when client is disconnected)
   */
  destroy(): void {
    if (this.dedupCleanupTimer) {
      clearInterval(this.dedupCleanupTimer);
      this.dedupCleanupTimer = undefined;
    }
    this.recentEventIds.clear();
  }

  /**
   * Remove expired event IDs from the deduplication cache
   * @internal
   */
  private cleanupExpiredEventIds(): void {
    const now = Date.now();
    for (const [eventId, expiration] of this.recentEventIds) {
      if (expiration <= now) {
        this.recentEventIds.delete(eventId);
      }
    }
  }

  /**
   * Track an event ID for deduplication
   * @internal
   */
  private trackEventId(eventId: string): void {
    const expiration = Date.now() + this.dedupWindowMs;
    this.recentEventIds.set(eventId, expiration);
  }

  /**
   * Check if an event ID has been seen recently
   * @internal
   */
  private isEventDuplicate(eventId: string): boolean {
    const expiration = this.recentEventIds.get(eventId);
    if (!expiration) return false;
    return expiration > Date.now();
  }

  /**
   * Create a deterministic hash of subscription filters
   *
   * Two subscriptions with identical filters will produce the same hash,
   * allowing deduplication.
   */
  private hashFilters(filters: SubscriptionFilters): string {
    // Sort event_types for consistent hashing
    const sortedEventTypes = filters.event_types
      ? [...filters.event_types].sort()
      : undefined;

    // Create deterministic JSON string
    return JSON.stringify({
      workspace: filters.workspace ?? null,
      path: filters.path ?? null,
      event_types: sortedEventTypes ?? null,
      node_type: filters.node_type ?? null,
      include_node: filters.include_node ?? false,
    });
  }

  /**
   * Subscribe to events with filters
   *
   * If a subscription with identical filters already exists, the callback
   * is added to the existing subscription instead of creating a new one.
   *
   * @param filters - Event filters
   * @param callback - Callback function for events
   * @returns Subscription object
   */
  async subscribe(
    filters: SubscriptionFilters,
    callback: EventCallback
  ): Promise<Subscription> {
    const hash = this.hashFilters(filters);

    // Check for existing subscription with same filters
    const existing = this.filterSubscriptions.get(hash);
    if (existing) {
      logger.debug('Reusing existing subscription with identical filters');
      existing.callbacks.add(callback);
      this.callbackToHash.set(callback, hash);

      return this.createSubscriptionHandle(existing.serverId, callback, hash);
    }

    // Create new server subscription
    const payload: SubscribePayload = { filters };
    const response = (await this.sendRequest(
      payload,
      RequestType.Subscribe
    )) as SubscriptionResponse;

    const serverId = response.subscription_id;

    // Track the new subscription
    this.filterSubscriptions.set(hash, {
      serverId,
      filters,
      callbacks: new Set([callback]),
    });
    this.subscriptions.set(serverId, hash);
    this.callbackToHash.set(callback, hash);

    return this.createSubscriptionHandle(serverId, callback, hash);
  }

  /**
   * Create a subscription handle for the callback
   */
  private createSubscriptionHandle(
    serverId: string,
    callback: EventCallback,
    hash: string
  ): Subscription {
    return {
      id: serverId,
      unsubscribe: async () => {
        await this.unsubscribeCallback(callback);
      },
      isActive: () => {
        const entry = this.filterSubscriptions.get(hash);
        return entry?.callbacks.has(callback) ?? false;
      },
    };
  }

  /**
   * Unsubscribe a specific callback
   *
   * Only sends unsubscribe to server when the last callback is removed.
   */
  private async unsubscribeCallback(callback: EventCallback): Promise<void> {
    const hash = this.callbackToHash.get(callback);
    if (!hash) {
      logger.warn('Callback not found in subscription map');
      return;
    }

    const entry = this.filterSubscriptions.get(hash);
    if (!entry) {
      logger.warn('Subscription entry not found for hash');
      return;
    }

    // Remove this callback
    entry.callbacks.delete(callback);
    this.callbackToHash.delete(callback);

    // If no more callbacks, unsubscribe from server
    if (entry.callbacks.size === 0) {
      const payload: UnsubscribePayload = { subscription_id: entry.serverId };
      await this.sendRequest(payload, RequestType.Unsubscribe);

      this.filterSubscriptions.delete(hash);
      this.subscriptions.delete(entry.serverId);
    }
  }

  /**
   * Unsubscribe from events by subscription ID
   *
   * @param subscriptionId - Subscription ID to cancel
   * @deprecated Use subscription.unsubscribe() instead
   */
  async unsubscribe(subscriptionId: string): Promise<void> {
    const hash = this.subscriptions.get(subscriptionId);
    if (!hash) {
      // Fall back to direct unsubscribe for backwards compatibility
      const payload: UnsubscribePayload = { subscription_id: subscriptionId };
      await this.sendRequest(payload, RequestType.Unsubscribe);
      return;
    }

    const entry = this.filterSubscriptions.get(hash);
    if (!entry) {
      return;
    }

    // Remove all callbacks and unsubscribe from server
    for (const callback of entry.callbacks) {
      this.callbackToHash.delete(callback);
    }
    entry.callbacks.clear();

    const payload: UnsubscribePayload = { subscription_id: subscriptionId };
    await this.sendRequest(payload, RequestType.Unsubscribe);

    this.filterSubscriptions.delete(hash);
    this.subscriptions.delete(subscriptionId);
  }

  /**
   * Handle incoming event message
   *
   * Routes event to all callbacks registered for the subscription.
   * Automatically deduplicates events using event_id within a 5-second window.
   *
   * @param event - Event message from server
   */
  handleEvent(event: EventMessage): void {
    // Deduplicate events using event_id
    if (event.event_id && this.isEventDuplicate(event.event_id)) {
      logger.debug('Skipping duplicate event:', event.event_id);
      return;
    }

    // Track this event for deduplication
    if (event.event_id) {
      this.trackEventId(event.event_id);
    }

    const hash = this.subscriptions.get(event.subscription_id);
    if (!hash) {
      logger.warn('Received event for unknown subscription:', event.subscription_id);
      return;
    }

    const entry = this.filterSubscriptions.get(hash);
    if (!entry) {
      logger.warn('No entry found for subscription hash');
      return;
    }

    // Call all registered callbacks
    for (const callback of entry.callbacks) {
      try {
        callback(event);
      } catch (error) {
        logger.error('Error in event callback:', error);
      }
    }
  }

  /**
   * Get all active subscription IDs
   */
  getActiveSubscriptions(): string[] {
    return Array.from(this.subscriptions.keys());
  }

  /**
   * Check if a subscription is active
   *
   * @param subscriptionId - Subscription ID to check
   */
  hasSubscription(subscriptionId: string): boolean {
    return this.subscriptions.has(subscriptionId);
  }

  /**
   * Clear all subscriptions (used during disconnect)
   */
  clearAll(): void {
    this.subscriptions.clear();
    this.filterSubscriptions.clear();
    this.callbackToHash.clear();
  }

  /**
   * Check if there are stored filters that need restoration
   * Used to detect if we need to restore subscriptions on reconnect
   */
  hasStoredFilters(): boolean {
    return this.filterSubscriptions.size > 0 || this.callbackToHash.size > 0;
  }

  /**
   * Register a direct listener for flow events by subscription ID.
   * Used by FlowsApi to receive flow execution events via WS.
   */
  addFlowEventListener(subscriptionId: string, callback: EventCallback): void {
    // Create a synthetic entry so handleEvent routes events to this callback
    const hash = `__flow__${subscriptionId}`;
    this.filterSubscriptions.set(hash, {
      serverId: subscriptionId,
      filters: {} as SubscriptionFilters,
      callbacks: new Set([callback]),
    });
    this.subscriptions.set(subscriptionId, hash);
    this.callbackToHash.set(callback, hash);
  }

  /**
   * Remove a flow event listener by subscription ID.
   */
  removeFlowEventListener(subscriptionId: string): void {
    const hash = this.subscriptions.get(subscriptionId);
    if (!hash) return;

    const entry = this.filterSubscriptions.get(hash);
    if (entry) {
      for (const callback of entry.callbacks) {
        this.callbackToHash.delete(callback);
      }
      this.filterSubscriptions.delete(hash);
    }
    this.subscriptions.delete(subscriptionId);
  }

  /**
   * Restore subscriptions after reconnect
   *
   * Re-subscribes to the server with the original filters for all active
   * subscription entries.
   */
  async restoreSubscriptions(): Promise<void> {
    // Collect entries to restore (can't modify map while iterating)
    const entriesToRestore = Array.from(this.filterSubscriptions.entries());

    // Clear current state
    this.subscriptions.clear();
    this.filterSubscriptions.clear();
    // Keep callbackToHash intact for callback lookups

    // Re-subscribe each entry
    for (const [hash, entry] of entriesToRestore) {
      if (entry.callbacks.size === 0) continue;

      try {
        const payload: SubscribePayload = { filters: entry.filters };
        const response = (await this.sendRequest(
          payload,
          RequestType.Subscribe
        )) as SubscriptionResponse;

        const newServerId = response.subscription_id;

        // Update mappings with new server ID
        this.filterSubscriptions.set(hash, {
          serverId: newServerId,
          filters: entry.filters,
          callbacks: entry.callbacks,
        });
        this.subscriptions.set(newServerId, hash);

        // Update callbackToHash (hash stays the same, just verify it's set)
        for (const callback of entry.callbacks) {
          this.callbackToHash.set(callback, hash);
        }
      } catch (error) {
        logger.error('Failed to restore subscription:', error);
        // Remove callbacks that couldn't be restored
        for (const callback of entry.callbacks) {
          this.callbackToHash.delete(callback);
        }
      }
    }
  }
}

/**
 * Event subscriptions interface for workspace
 */
export class EventSubscriptions {
  private eventHandler: EventHandler;
  private filters: Partial<SubscriptionFilters>;

  constructor(eventHandler: EventHandler, filters: Partial<SubscriptionFilters> = {}) {
    this.eventHandler = eventHandler;
    this.filters = filters;
  }

  /**
   * Subscribe to events
   *
   * @param filters - Additional filters (merged with workspace filters)
   * @param callback - Callback function for events
   * @returns Subscription object
   */
  async subscribe(
    filters: Partial<SubscriptionFilters>,
    callback: EventCallback
  ): Promise<Subscription> {
    // Merge workspace filters with provided filters
    const mergedFilters: SubscriptionFilters = {
      ...this.filters,
      ...filters,
    } as SubscriptionFilters;

    return this.eventHandler.subscribe(mergedFilters, callback);
  }

  /**
   * Subscribe to specific event types
   *
   * @param eventTypes - Event types to subscribe to
   * @param callback - Callback function for events
   * @returns Subscription object
   */
  async subscribeToTypes(eventTypes: string[], callback: EventCallback): Promise<Subscription> {
    return this.subscribe({ event_types: eventTypes }, callback);
  }

  /**
   * Subscribe to events for a specific path
   *
   * @param path - Path pattern (supports wildcards like "/folder/*")
   * @param callback - Callback function for events
   * @param options - Optional subscription options
   * @param options.includeNode - Include full node data in event payload (default: false)
   * @returns Subscription object
   */
  async subscribeToPath(
    path: string,
    callback: EventCallback,
    options?: { includeNode?: boolean }
  ): Promise<Subscription> {
    const filters: Partial<SubscriptionFilters> = { path };
    if (options?.includeNode) {
      filters.include_node = true;
    }
    return this.subscribe(filters, callback);
  }

  /**
   * Subscribe to events for a specific node type
   *
   * @param nodeType - Node type to filter by
   * @param callback - Callback function for events
   * @returns Subscription object
   */
  async subscribeToNodeType(nodeType: string, callback: EventCallback): Promise<Subscription> {
    return this.subscribe({ node_type: nodeType }, callback);
  }
}
