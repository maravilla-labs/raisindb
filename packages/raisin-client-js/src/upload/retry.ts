/**
 * Retry utility with exponential backoff and jitter
 *
 * Implements a robust retry strategy for handling transient failures
 * in upload operations.
 */

import {
  RetryOptions,
  UploadError,
  UploadErrorCode,
  DEFAULT_MAX_RETRIES,
  DEFAULT_RETRY_BASE_DELAY,
  DEFAULT_RETRY_MAX_DELAY,
} from './types';

// ============================================================================
// Default Options
// ============================================================================

/**
 * Default retry options
 */
export const DEFAULT_RETRY_OPTIONS: RetryOptions = {
  maxRetries: DEFAULT_MAX_RETRIES,
  baseDelay: DEFAULT_RETRY_BASE_DELAY,
  maxDelay: DEFAULT_RETRY_MAX_DELAY,
  exponentialBackoff: true,
};

// ============================================================================
// Retry Context
// ============================================================================

/**
 * Context passed to retry callbacks
 */
export interface RetryContext {
  /** Current attempt number (1-indexed) */
  attempt: number;
  /** Total attempts allowed */
  maxAttempts: number;
  /** Last error that occurred */
  lastError: Error | null;
  /** Whether this is a retry (not the first attempt) */
  isRetry: boolean;
}

/**
 * Callback for retry events
 */
export type RetryCallback = (context: RetryContext) => void;

// ============================================================================
// Delay Calculation
// ============================================================================

/**
 * Calculate delay with exponential backoff and jitter
 *
 * Uses the formula: delay = min(maxDelay, baseDelay * 2^attempt + jitter)
 * Jitter is a random value between 0 and 30% of the base delay.
 *
 * @param attempt - Current attempt number (0-indexed)
 * @param options - Retry options
 * @returns Delay in milliseconds
 */
export function calculateDelay(attempt: number, options: RetryOptions): number {
  const { baseDelay, maxDelay, exponentialBackoff } = options;

  let delay: number;

  if (exponentialBackoff) {
    // Exponential: baseDelay * 2^attempt
    delay = baseDelay * Math.pow(2, attempt);
  } else {
    // Linear: just use base delay
    delay = baseDelay;
  }

  // Add jitter (0-30% of base delay)
  const jitter = Math.random() * 0.3 * baseDelay;
  delay += jitter;

  // Cap at max delay
  return Math.min(delay, maxDelay);
}

/**
 * Sleep for a given number of milliseconds
 *
 * @param ms - Milliseconds to sleep
 * @param signal - Optional abort signal
 * @returns Promise that resolves after the delay
 * @throws UploadError if aborted
 */
export async function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    // Check if already aborted
    if (signal?.aborted) {
      reject(UploadError.cancelledError());
      return;
    }

    const timeoutId = setTimeout(resolve, ms);

    // Listen for abort
    if (signal) {
      const onAbort = () => {
        clearTimeout(timeoutId);
        reject(UploadError.cancelledError());
      };

      signal.addEventListener('abort', onAbort, { once: true });

      // Clean up listener after timeout
      setTimeout(() => {
        signal.removeEventListener('abort', onAbort);
      }, ms);
    }
  });
}

// ============================================================================
// Retry Functions
// ============================================================================

/**
 * Determine if an error is retryable
 *
 * @param error - Error to check
 * @returns True if the error can be retried
 */
export function isRetryableError(error: unknown): boolean {
  // UploadError has explicit retryable flag
  if (error instanceof UploadError) {
    return error.retryable;
  }

  // Check for common retryable error patterns
  if (error instanceof Error) {
    const message = error.message.toLowerCase();

    // Network errors
    if (
      message.includes('network') ||
      message.includes('fetch') ||
      message.includes('connection') ||
      message.includes('econnreset') ||
      message.includes('econnrefused') ||
      message.includes('etimedout') ||
      message.includes('timeout')
    ) {
      return true;
    }

    // Check error name/type
    const name = error.name.toLowerCase();
    if (name === 'networkerror' || name === 'typeerror') {
      // TypeError can occur with fetch on network failures
      return true;
    }
  }

  return false;
}

/**
 * Execute a function with retry logic
 *
 * @param fn - Async function to execute
 * @param options - Retry options
 * @param signal - Optional abort signal
 * @param onRetry - Optional callback for retry events
 * @returns Result of the function
 * @throws Last error if all retries are exhausted
 */
export async function withRetry<T>(
  fn: () => Promise<T>,
  options: Partial<RetryOptions> = {},
  signal?: AbortSignal,
  onRetry?: RetryCallback
): Promise<T> {
  const opts: RetryOptions = { ...DEFAULT_RETRY_OPTIONS, ...options };
  const maxAttempts = opts.maxRetries + 1; // Total attempts = retries + 1

  let lastError: Error | null = null;

  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    // Check for abort
    if (signal?.aborted) {
      throw UploadError.cancelledError();
    }

    try {
      return await fn();
    } catch (error) {
      lastError = error instanceof Error ? error : new Error(String(error));

      // Don't retry if not retryable
      if (!isRetryableError(error)) {
        throw error;
      }

      // Don't retry if this was the last attempt
      if (attempt >= maxAttempts - 1) {
        throw error;
      }

      // Calculate delay
      const delay = calculateDelay(attempt, opts);

      // Notify retry callback
      if (onRetry) {
        onRetry({
          attempt: attempt + 1,
          maxAttempts,
          lastError,
          isRetry: true,
        });
      }

      // Wait before next attempt
      await sleep(delay, signal);
    }
  }

  // Should not reach here, but TypeScript needs a return
  throw lastError ?? new Error('Retry failed');
}

// ============================================================================
// Retry Builder (Fluent API)
// ============================================================================

/**
 * Builder for creating retry configurations
 *
 * @example
 * ```typescript
 * const result = await new RetryBuilder()
 *   .maxRetries(5)
 *   .baseDelay(2000)
 *   .onRetry((ctx) => console.log(`Retry ${ctx.attempt}/${ctx.maxAttempts}`))
 *   .execute(() => uploadChunk(data));
 * ```
 */
export class RetryBuilder {
  private options: RetryOptions = { ...DEFAULT_RETRY_OPTIONS };
  private signal?: AbortSignal;
  private retryCallback?: RetryCallback;

  /**
   * Set maximum number of retries
   */
  maxRetries(count: number): this {
    this.options.maxRetries = count;
    return this;
  }

  /**
   * Set base delay in milliseconds
   */
  baseDelay(ms: number): this {
    this.options.baseDelay = ms;
    return this;
  }

  /**
   * Set maximum delay in milliseconds
   */
  maxDelay(ms: number): this {
    this.options.maxDelay = ms;
    return this;
  }

  /**
   * Enable or disable exponential backoff
   */
  exponentialBackoff(enabled: boolean): this {
    this.options.exponentialBackoff = enabled;
    return this;
  }

  /**
   * Set abort signal
   */
  withSignal(signal: AbortSignal): this {
    this.signal = signal;
    return this;
  }

  /**
   * Set retry callback
   */
  onRetry(callback: RetryCallback): this {
    this.retryCallback = callback;
    return this;
  }

  /**
   * Execute a function with retry
   */
  async execute<T>(fn: () => Promise<T>): Promise<T> {
    return withRetry(fn, this.options, this.signal, this.retryCallback);
  }
}

// ============================================================================
// Error Classification
// ============================================================================

/**
 * Classify an error for retry decisions
 */
export interface ErrorClassification {
  /** Error code */
  code: UploadErrorCode;
  /** Whether the error is retryable */
  retryable: boolean;
  /** Suggested delay multiplier for this error type */
  delayMultiplier: number;
  /** Human-readable description */
  description: string;
}

/**
 * Classify an error for retry decisions
 *
 * @param error - Error to classify
 * @returns Error classification
 */
export function classifyError(error: unknown): ErrorClassification {
  if (error instanceof UploadError) {
    return {
      code: error.code,
      retryable: error.retryable,
      delayMultiplier: getDelayMultiplier(error.code),
      description: error.message,
    };
  }

  // Handle generic errors
  if (error instanceof Error) {
    const message = error.message.toLowerCase();

    // Timeout errors - moderate backoff
    if (message.includes('timeout')) {
      return {
        code: UploadErrorCode.TIMEOUT,
        retryable: true,
        delayMultiplier: 1.5,
        description: 'Request timed out',
      };
    }

    // Network errors - aggressive backoff
    if (
      message.includes('network') ||
      message.includes('connection') ||
      message.includes('econnreset')
    ) {
      return {
        code: UploadErrorCode.NETWORK_ERROR,
        retryable: true,
        delayMultiplier: 2.0,
        description: 'Network connection error',
      };
    }
  }

  // Unknown error - conservative approach
  return {
    code: UploadErrorCode.SERVER_ERROR,
    retryable: false,
    delayMultiplier: 1.0,
    description: 'Unknown error',
  };
}

/**
 * Get delay multiplier for an error code
 */
function getDelayMultiplier(code: UploadErrorCode): number {
  switch (code) {
    case UploadErrorCode.NETWORK_ERROR:
      return 2.0; // Network issues - wait longer
    case UploadErrorCode.TIMEOUT:
      return 1.5; // Timeouts - moderate wait
    case UploadErrorCode.SERVER_ERROR:
      return 1.5; // Server overload - moderate wait
    case UploadErrorCode.STORAGE_ERROR:
      return 1.0; // Storage issues - normal wait
    default:
      return 1.0;
  }
}
