/**
 * Error types for the RaisinDB JS SDK.
 *
 * Provides a structured error hierarchy for programmatic error handling.
 * All errors extend RaisinError and include a string `code` field for
 * easy matching in switch statements or error handlers.
 *
 * @example
 * ```typescript
 * import { RaisinError, RaisinConnectionError } from '@raisindb/client';
 *
 * try {
 *   await flows.run('/flows/my-flow', {});
 * } catch (err) {
 *   if (err instanceof RaisinConnectionError) {
 *     console.log('Network issue:', err.code);
 *   }
 * }
 * ```
 */

// ============================================================================
// Error Codes
// ============================================================================

/** Error codes for connection errors */
export type ConnectionErrorCode =
  | 'CONNECTION_FAILED'
  | 'CONNECTION_LOST'
  | 'SSE_STREAM_ERROR';

/** Error codes for authentication errors */
export type AuthErrorCode =
  | 'AUTH_UNAUTHORIZED'
  | 'AUTH_FORBIDDEN'
  | 'AUTH_TOKEN_EXPIRED';

/** Error codes for flow execution errors */
export type FlowErrorCode =
  | 'FLOW_NOT_FOUND'
  | 'FLOW_EXECUTION_FAILED'
  | 'FLOW_INSTANCE_NOT_FOUND'
  | 'FLOW_RESUME_FAILED';

/** Error codes for timeout errors */
export type TimeoutErrorCode =
  | 'REQUEST_TIMEOUT'
  | 'CONNECTION_TIMEOUT'
  | 'POLL_TIMEOUT';

/** Error codes for abort/cancellation errors */
export type AbortErrorCode = 'ABORTED';

/** All error codes */
export type RaisinErrorCode =
  | ConnectionErrorCode
  | AuthErrorCode
  | FlowErrorCode
  | TimeoutErrorCode
  | AbortErrorCode;

// ============================================================================
// Error Classes
// ============================================================================

/**
 * Base error class for all RaisinDB SDK errors.
 *
 * All SDK errors extend this class, making it easy to catch any
 * RaisinDB-specific error with a single `instanceof` check.
 */
export class RaisinError extends Error {
  /** String error code for programmatic handling */
  readonly code: RaisinErrorCode;
  /** Optional additional details */
  readonly details?: unknown;

  constructor(message: string, code: RaisinErrorCode, details?: unknown) {
    super(message);
    this.name = 'RaisinError';
    this.code = code;
    this.details = details;
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, this.constructor);
    }
  }
}

/**
 * Thrown when a network or connection failure occurs.
 *
 * This includes SSE stream errors, fetch failures, and lost connections.
 */
export class RaisinConnectionError extends RaisinError {
  /** HTTP status code, if available */
  readonly status?: number;

  constructor(message: string, code: ConnectionErrorCode, options?: { status?: number; details?: unknown }) {
    super(message, code, options?.details);
    this.name = 'RaisinConnectionError';
    this.status = options?.status;
  }
}

/**
 * Thrown when an authentication or authorization failure occurs.
 *
 * This includes 401 Unauthorized, 403 Forbidden, and expired tokens.
 */
export class RaisinAuthError extends RaisinError {
  /** HTTP status code (401 or 403) */
  readonly status: number;

  constructor(message: string, code: AuthErrorCode, status: number, details?: unknown) {
    super(message, code, details);
    this.name = 'RaisinAuthError';
    this.status = status;
  }
}

/**
 * Thrown when a flow execution error occurs.
 *
 * This includes flow not found, execution failures, and resume errors.
 */
export class RaisinFlowError extends RaisinError {
  /** Flow instance ID, if available */
  readonly instanceId?: string;

  constructor(message: string, code: FlowErrorCode, instanceId?: string, details?: unknown) {
    super(message, code, details);
    this.name = 'RaisinFlowError';
    this.instanceId = instanceId;
  }
}

/**
 * Thrown when a request or operation times out.
 *
 * Includes the timeout duration for debugging.
 */
export class RaisinTimeoutError extends RaisinError {
  /** Timeout duration in milliseconds */
  readonly timeoutMs: number;

  constructor(message: string, code: TimeoutErrorCode, timeoutMs: number, details?: unknown) {
    super(message, code, details);
    this.name = 'RaisinTimeoutError';
    this.timeoutMs = timeoutMs;
  }
}

/**
 * Thrown when an operation is aborted via AbortSignal.
 */
export class RaisinAbortError extends RaisinError {
  constructor(message = 'Operation was aborted') {
    super(message, 'ABORTED');
    this.name = 'RaisinAbortError';
  }
}

// ============================================================================
// Helpers
// ============================================================================

/**
 * Classify an HTTP response error into the appropriate RaisinError subclass.
 *
 * @internal
 */
export function classifyHttpError(
  status: number,
  message: string,
  details?: unknown,
): RaisinError {
  if (status === 401) {
    return new RaisinAuthError(message, 'AUTH_UNAUTHORIZED', status, details);
  }
  if (status === 403) {
    return new RaisinAuthError(message, 'AUTH_FORBIDDEN', status, details);
  }
  if (status === 404) {
    return new RaisinFlowError(message, 'FLOW_NOT_FOUND', undefined, details);
  }
  return new RaisinConnectionError(message, 'CONNECTION_FAILED', { status, details });
}
