/**
 * Request tracker for managing pending requests and their promises
 */

import { v4 as uuidv4 } from 'uuid';

/**
 * Pending request information
 */
interface PendingRequest<T = unknown> {
  resolve: (value: T) => void;
  reject: (error: Error) => void;
  timeout?: NodeJS.Timeout;
}

/**
 * Request tracker options
 */
export interface RequestTrackerOptions {
  /** Default timeout in milliseconds (default: 30000) */
  defaultTimeout?: number;
}

/**
 * RequestTracker manages pending requests and their promises
 */
export class RequestTracker {
  private pendingRequests = new Map<string, PendingRequest>();
  private defaultTimeout: number;

  constructor(options: RequestTrackerOptions = {}) {
    this.defaultTimeout = options.defaultTimeout ?? 30000;
  }

  /**
   * Generate a new unique request ID
   */
  generateRequestId(): string {
    return uuidv4();
  }

  /**
   * Create a new tracked request
   *
   * @param requestId - Unique request ID
   * @param timeout - Timeout in milliseconds (uses default if not specified)
   * @returns Promise that resolves when response is received
   */
  createRequest<T = unknown>(requestId: string, timeout?: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timeoutMs = timeout ?? this.defaultTimeout;

      // Set up timeout
      const timeoutHandle = setTimeout(() => {
        this.rejectRequest(
          requestId,
          new Error(`Request ${requestId} timed out after ${timeoutMs}ms`)
        );
      }, timeoutMs);

      // Store pending request
      this.pendingRequests.set(requestId, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout: timeoutHandle,
      });
    });
  }

  /**
   * Resolve a pending request
   *
   * @param requestId - Request ID to resolve
   * @param value - Value to resolve with
   */
  resolveRequest<T = unknown>(requestId: string, value: T): void {
    const pending = this.pendingRequests.get(requestId);
    if (pending) {
      if (pending.timeout) {
        clearTimeout(pending.timeout);
      }
      pending.resolve(value);
      this.pendingRequests.delete(requestId);
    }
  }

  /**
   * Reject a pending request
   *
   * @param requestId - Request ID to reject
   * @param error - Error to reject with
   */
  rejectRequest(requestId: string, error: Error): void {
    const pending = this.pendingRequests.get(requestId);
    if (pending) {
      if (pending.timeout) {
        clearTimeout(pending.timeout);
      }
      pending.reject(error);
      this.pendingRequests.delete(requestId);
    }
  }

  /**
   * Cancel a pending request
   *
   * @param requestId - Request ID to cancel
   */
  cancelRequest(requestId: string): void {
    this.rejectRequest(requestId, new Error(`Request ${requestId} was cancelled`));
  }

  /**
   * Check if a request is pending
   *
   * @param requestId - Request ID to check
   */
  hasPendingRequest(requestId: string): boolean {
    return this.pendingRequests.has(requestId);
  }

  /**
   * Get the number of pending requests
   */
  getPendingCount(): number {
    return this.pendingRequests.size;
  }

  /**
   * Cancel all pending requests
   */
  cancelAll(): void {
    const requestIds = Array.from(this.pendingRequests.keys());
    for (const requestId of requestIds) {
      this.cancelRequest(requestId);
    }
  }

  /**
   * Clear all pending requests without rejecting them
   * (useful during cleanup)
   */
  clear(): void {
    for (const pending of this.pendingRequests.values()) {
      if (pending.timeout) {
        clearTimeout(pending.timeout);
      }
    }
    this.pendingRequests.clear();
  }
}
