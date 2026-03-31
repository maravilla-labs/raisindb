/**
 * Reconnection manager with exponential backoff
 */

/**
 * Reconnection strategy options
 */
export interface ReconnectOptions {
  /** Initial delay in milliseconds (default: 1000) */
  initialDelay?: number;
  /** Maximum delay in milliseconds (default: 30000) */
  maxDelay?: number;
  /** Backoff multiplier (default: 2) */
  backoffMultiplier?: number;
  /** Maximum number of reconnection attempts (default: Infinity) */
  maxAttempts?: number;
  /** Add random jitter to delay (default: true). Jitter is +/- 25% of the computed delay. */
  jitter?: boolean;
}

/**
 * Reconnection manager with exponential backoff
 */
export class ReconnectManager {
  private currentDelay: number;
  private attempts = 0;
  private reconnectTimeout?: NodeJS.Timeout;
  private isReconnecting = false;

  private readonly initialDelay: number;
  private readonly maxDelay: number;
  private readonly backoffMultiplier: number;
  private readonly maxAttempts: number;
  private readonly jitter: boolean;

  constructor(options: ReconnectOptions = {}) {
    this.initialDelay = options.initialDelay ?? 1000;
    this.maxDelay = options.maxDelay ?? 30000;
    this.backoffMultiplier = options.backoffMultiplier ?? 2;
    this.maxAttempts = options.maxAttempts ?? Infinity;
    this.jitter = options.jitter ?? true;
    this.currentDelay = this.initialDelay;
  }

  /**
   * Schedule a reconnection attempt
   *
   * @param callback - Function to call when it's time to reconnect
   * @returns true if reconnection was scheduled, false if max attempts reached
   */
  scheduleReconnect(callback: () => void | Promise<void>): boolean {
    // Check if we've exceeded max attempts
    if (this.attempts >= this.maxAttempts) {
      return false;
    }

    // Clear any existing timeout
    this.cancelReconnect();

    this.isReconnecting = true;
    this.attempts++;

    // Apply jitter: +/- 25% of the computed delay
    let delay = this.currentDelay;
    if (this.jitter) {
      const jitterRange = delay * 0.25;
      delay += (Math.random() * 2 - 1) * jitterRange;
      delay = Math.max(0, Math.round(delay));
    }

    this.reconnectTimeout = setTimeout(async () => {
      try {
        await callback();
        // If successful, reset the delay
        this.reset();
      } catch (error) {
        // Increase delay for next attempt
        this.currentDelay = Math.min(
          this.currentDelay * this.backoffMultiplier,
          this.maxDelay
        );
      }
    }, delay);

    return true;
  }

  /**
   * Cancel any pending reconnection attempt
   */
  cancelReconnect(): void {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = undefined;
    }
    this.isReconnecting = false;
  }

  /**
   * Reset the reconnection state (call after successful connection)
   */
  reset(): void {
    this.cancelReconnect();
    this.currentDelay = this.initialDelay;
    this.attempts = 0;
  }

  /**
   * Check if currently in reconnection mode
   */
  isActive(): boolean {
    return this.isReconnecting;
  }

  /**
   * Get the current number of reconnection attempts
   */
  getAttempts(): number {
    return this.attempts;
  }

  /**
   * Get the current delay in milliseconds
   */
  getCurrentDelay(): number {
    return this.currentDelay;
  }

  /**
   * Check if max attempts have been reached
   */
  hasReachedMaxAttempts(): boolean {
    return this.attempts >= this.maxAttempts;
  }
}
