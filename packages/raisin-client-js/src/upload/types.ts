/**
 * Upload type definitions for resumable chunked uploads
 *
 * This module defines types for the resumable upload system supporting
 * files up to 10GB+ with progress tracking, pause/resume, and auto-retry.
 */

import type { Node } from '../protocol';

// ============================================================================
// Upload Status
// ============================================================================

/**
 * Possible states of an upload
 */
export type UploadStatus =
  | 'pending'
  | 'uploading'
  | 'paused'
  | 'completing'
  | 'completed'
  | 'failed'
  | 'cancelled';

// ============================================================================
// Upload Options
// ============================================================================

/**
 * Options for creating a resumable upload
 */
export interface UploadOptions {
  /** Repository to upload to */
  repository: string;
  /** Branch name (default: "main") */
  branch?: string;
  /** Workspace to store the node */
  workspace: string;
  /** Path for the new node */
  path: string;
  /** Node type (default: "raisin:Asset") */
  nodeType?: string;
  /** Additional metadata for the node */
  metadata?: Record<string, unknown>;
  /** Chunk size in bytes (default: 10MB) */
  chunkSize?: number;
  /** Progress callback */
  onProgress?: (progress: UploadProgress) => void;
  /** Abort signal for cancellation */
  signal?: AbortSignal;
  /** Commit message for the upload */
  commitMessage?: string;
  /** Actor/user identifier for the commit */
  commitActor?: string;
  /** Enable automatic retry on retryable errors (default: true) */
  autoRetry?: boolean;
  /** Maximum number of retry attempts (default: 3) */
  maxRetries?: number;
}

// ============================================================================
// Upload Progress
// ============================================================================

/**
 * Progress information for an upload
 */
export interface UploadProgress {
  /** Bytes uploaded so far */
  bytesUploaded: number;
  /** Total bytes to upload */
  bytesTotal: number;
  /** Progress as a fraction (0-1) */
  progress: number;
  /** Current upload speed in bytes/second */
  speed: number;
  /** Estimated time remaining in seconds */
  eta: number;
  /** Current chunk being uploaded (1-indexed) */
  currentChunk: number;
  /** Total number of chunks */
  totalChunks: number;
  /** Current upload status */
  status: UploadStatus;
}

// ============================================================================
// Upload Result
// ============================================================================

/**
 * Result of a completed upload
 */
export interface UploadResult {
  /** The created node */
  node: Node;
  /** Revision/commit ID */
  revision?: string;
  /** Storage key for the binary content */
  storageKey: string;
}

// ============================================================================
// Upload Interface
// ============================================================================

/**
 * Interface for controlling an upload
 */
export interface Upload {
  /** Unique upload ID */
  id: string;
  /** Start or resume the upload */
  start(): Promise<UploadResult>;
  /** Pause the upload */
  pause(): void;
  /** Resume a paused upload */
  resume(): Promise<UploadResult>;
  /** Cancel the upload */
  cancel(): Promise<void>;
  /** Get current progress */
  getProgress(): UploadProgress;
  /** Check if the upload is currently active */
  isActive(): boolean;
}

// ============================================================================
// API Request/Response Types
// ============================================================================

/**
 * Request payload for creating an upload session
 */
export interface CreateUploadRequest {
  /** Repository name */
  repository: string;
  /** Branch name */
  branch: string;
  /** Workspace name */
  workspace: string;
  /** Node path */
  path: string;
  /** Original filename */
  filename: string;
  /** Total file size in bytes */
  file_size: number;
  /** Content MIME type */
  content_type?: string;
  /** Node type for the created node */
  node_type?: string;
  /** Chunk size in bytes */
  chunk_size?: number;
  /** Additional metadata */
  metadata?: Record<string, unknown>;
}

/**
 * Response from creating an upload session
 */
export interface CreateUploadResponse {
  /** Unique upload session ID */
  upload_id: string;
  /** URL for uploading chunks */
  upload_url: string;
  /** Chunk size determined by server */
  chunk_size: number;
  /** Total number of chunks expected */
  total_chunks: number;
  /** Session expiration time */
  expires_at: string;
}

/**
 * Response from uploading a chunk
 */
export interface ChunkUploadResponse {
  /** Upload session ID */
  upload_id: string;
  /** Total bytes received so far */
  bytes_received: number;
  /** Total bytes expected */
  bytes_total: number;
  /** Number of chunks completed */
  chunks_completed: number;
  /** Total number of chunks */
  chunks_total: number;
  /** Progress as a fraction (0-1) */
  progress: number;
}

/**
 * Response from getting upload status
 */
export interface UploadStatusResponse {
  /** Upload session ID */
  upload_id: string;
  /** Session status */
  status: string;
  /** Bytes received so far */
  bytes_received: number;
  /** Total bytes expected */
  bytes_total: number;
  /** Chunks completed */
  chunks_completed: number;
  /** Total chunks */
  chunks_total: number;
  /** Session expiration time */
  expires_at: string;
}

/**
 * Request payload for completing an upload
 */
export interface CompleteUploadRequest {
  /** Optional commit message */
  commit_message?: string;
  /** Optional commit actor */
  commit_actor?: string;
}

/**
 * Response from completing an upload
 */
export interface CompleteUploadResponse {
  /** The created node */
  node: Node;
  /** Revision/commit ID */
  revision?: string;
  /** Storage key for the binary */
  storage_key: string;
}

// ============================================================================
// Retry Options
// ============================================================================

/**
 * Options for retry behavior
 */
export interface RetryOptions {
  /** Maximum number of retry attempts */
  maxRetries: number;
  /** Base delay in milliseconds */
  baseDelay: number;
  /** Maximum delay in milliseconds */
  maxDelay: number;
  /** Use exponential backoff */
  exponentialBackoff: boolean;
}

// ============================================================================
// Error Types
// ============================================================================

/**
 * Error codes for upload operations
 */
export enum UploadErrorCode {
  /** Invalid file (empty, too large, etc.) */
  INVALID_FILE = 'INVALID_FILE',
  /** File exceeds maximum size limit */
  FILE_TOO_LARGE = 'FILE_TOO_LARGE',
  /** Upload session has expired */
  SESSION_EXPIRED = 'SESSION_EXPIRED',
  /** Upload session not found */
  SESSION_NOT_FOUND = 'SESSION_NOT_FOUND',
  /** Chunk offset doesn't match expected position */
  CHUNK_OFFSET_MISMATCH = 'CHUNK_OFFSET_MISMATCH',
  /** Chunk checksum verification failed */
  CHECKSUM_MISMATCH = 'CHECKSUM_MISMATCH',
  /** Backend storage error */
  STORAGE_ERROR = 'STORAGE_ERROR',
  /** Server-side error */
  SERVER_ERROR = 'SERVER_ERROR',
  /** Network connectivity error */
  NETWORK_ERROR = 'NETWORK_ERROR',
  /** Request timeout */
  TIMEOUT = 'TIMEOUT',
  /** Upload was cancelled */
  CANCELLED = 'CANCELLED',
}

/**
 * Custom error class for upload operations
 */
export class UploadError extends Error {
  /**
   * Create an upload error
   *
   * @param message - Human-readable error message
   * @param code - Error code
   * @param retryable - Whether this error can be retried
   * @param details - Additional error details
   */
  constructor(
    message: string,
    public readonly code: UploadErrorCode,
    public readonly retryable: boolean,
    public readonly details?: unknown
  ) {
    super(message);
    this.name = 'UploadError';
    // Maintain proper stack trace in V8
    if (Error.captureStackTrace) {
      Error.captureStackTrace(this, UploadError);
    }
  }

  /**
   * Create a network error
   */
  static networkError(message: string, details?: unknown): UploadError {
    return new UploadError(message, UploadErrorCode.NETWORK_ERROR, true, details);
  }

  /**
   * Create a timeout error
   */
  static timeoutError(message: string, details?: unknown): UploadError {
    return new UploadError(message, UploadErrorCode.TIMEOUT, true, details);
  }

  /**
   * Create a cancelled error
   */
  static cancelledError(message = 'Upload was cancelled'): UploadError {
    return new UploadError(message, UploadErrorCode.CANCELLED, false);
  }

  /**
   * Create a session expired error
   */
  static sessionExpiredError(uploadId: string): UploadError {
    return new UploadError(
      `Upload session ${uploadId} has expired`,
      UploadErrorCode.SESSION_EXPIRED,
      false,
      { uploadId }
    );
  }

  /**
   * Create a session not found error
   */
  static sessionNotFoundError(uploadId: string): UploadError {
    return new UploadError(
      `Upload session ${uploadId} not found`,
      UploadErrorCode.SESSION_NOT_FOUND,
      false,
      { uploadId }
    );
  }

  /**
   * Create an error from an HTTP response
   */
  static fromResponse(status: number, body: unknown): UploadError {
    const message = typeof body === 'object' && body !== null && 'message' in body
      ? String((body as { message: unknown }).message)
      : `HTTP error ${status}`;

    // Map HTTP status to error code
    let code: UploadErrorCode;
    let retryable: boolean;

    switch (status) {
      case 400:
        code = UploadErrorCode.INVALID_FILE;
        retryable = false;
        break;
      case 404:
        code = UploadErrorCode.SESSION_NOT_FOUND;
        retryable = false;
        break;
      case 409:
        code = UploadErrorCode.CHUNK_OFFSET_MISMATCH;
        retryable = false;
        break;
      case 410:
        code = UploadErrorCode.SESSION_EXPIRED;
        retryable = false;
        break;
      case 413:
        code = UploadErrorCode.FILE_TOO_LARGE;
        retryable = false;
        break;
      case 500:
      case 502:
      case 503:
      case 504:
        code = UploadErrorCode.SERVER_ERROR;
        retryable = true;
        break;
      default:
        code = UploadErrorCode.SERVER_ERROR;
        retryable = status >= 500;
    }

    return new UploadError(message, code, retryable, body);
  }
}

// ============================================================================
// Default Values
// ============================================================================

/** Default chunk size: 10 MB */
export const DEFAULT_CHUNK_SIZE = 10 * 1024 * 1024;

/** Default maximum retries */
export const DEFAULT_MAX_RETRIES = 3;

/** Default base delay for retries: 1 second */
export const DEFAULT_RETRY_BASE_DELAY = 1000;

/** Default maximum delay for retries: 30 seconds */
export const DEFAULT_RETRY_MAX_DELAY = 30000;

/** Default node type for uploads */
export const DEFAULT_NODE_TYPE = 'raisin:Asset';

/** Default branch */
export const DEFAULT_BRANCH = 'main';

/** Default concurrency for batch uploads */
export const DEFAULT_BATCH_CONCURRENCY = 3;

// ============================================================================
// Batch Upload Types
// ============================================================================

/**
 * Progress for a single file in a batch upload
 */
export interface BatchFileProgress {
  /** Filename */
  file: string;
  /** Current upload status */
  status: UploadStatus;
  /** Bytes uploaded so far */
  bytesUploaded: number;
  /** Total bytes to upload */
  bytesTotal: number;
  /** Progress as a fraction (0-1) */
  progress: number;
  /** Error if upload failed */
  error?: Error;
}

/**
 * Aggregate progress across all files in a batch upload
 */
export interface BatchProgress {
  /** Total number of files in the batch */
  filesTotal: number;
  /** Number of files completed successfully */
  filesCompleted: number;
  /** Number of files that failed */
  filesFailed: number;
  /** Number of files currently uploading */
  filesInProgress: number;
  /** Number of files waiting in queue */
  filesPending: number;
  /** Total bytes uploaded across all files */
  bytesUploaded: number;
  /** Total bytes across all files */
  bytesTotal: number;
  /** Overall progress as a fraction (0-1) */
  progress: number;
  /** Aggregate upload speed in bytes/second */
  speed: number;
  /** Estimated time remaining in seconds */
  eta: number;
  /** Per-file progress details */
  files: BatchFileProgress[];
}

/**
 * Result of a completed batch upload
 */
export interface BatchUploadResult {
  /** Successfully uploaded files */
  successful: Array<{ file: string; result: UploadResult }>;
  /** Files that failed to upload */
  failed: Array<{ file: string; error: Error }>;
}

/**
 * Options for batch upload operations
 */
export interface BatchUploadOptions {
  /** Repository to upload to */
  repository: string;
  /** Branch name (default: "main") */
  branch?: string;
  /** Workspace to store the nodes */
  workspace: string;
  /** Base path - files will be uploaded to basePath/filename */
  basePath?: string;
  /** Custom function to determine path for each file */
  pathResolver?: (file: { name: string; size: number }) => string;
  /** Node type (default: "raisin:Asset") */
  nodeType?: string;
  /** Additional metadata for all nodes */
  metadata?: Record<string, unknown>;
  /** Chunk size in bytes (default: 10MB) */
  chunkSize?: number;
  /** Max concurrent uploads (default: 3) */
  concurrency?: number;
  /** Progress callback for aggregate progress */
  onProgress?: (progress: BatchProgress) => void;
  /** Called when individual file completes */
  onFileComplete?: (file: string, result: UploadResult) => void;
  /** Called when individual file fails */
  onFileError?: (file: string, error: Error) => void;
  /** Abort signal for cancellation */
  signal?: AbortSignal;
  /** Commit message for all uploads */
  commitMessage?: string;
  /** Actor/user identifier for all commits */
  commitActor?: string;
  /** Enable automatic retry on retryable errors (default: true) */
  autoRetry?: boolean;
  /** Maximum number of retry attempts per file (default: 3) */
  maxRetries?: number;
  /** Continue uploading remaining files if one fails (default: true) */
  continueOnError?: boolean;
}

/**
 * Interface for controlling a batch upload
 */
export interface BatchUpload {
  /** Start all uploads */
  start(): Promise<BatchUploadResult>;
  /** Pause all active uploads */
  pause(): void;
  /** Resume paused uploads */
  resume(): Promise<BatchUploadResult>;
  /** Cancel all uploads */
  cancel(): Promise<void>;
  /** Get current aggregate progress */
  getProgress(): BatchProgress;
  /** Check if any uploads are active */
  isActive(): boolean;
}
