/**
 * Upload module for RaisinDB client
 *
 * Provides resumable chunked uploads with:
 * - Progress tracking with speed and ETA
 * - Pause/resume support
 * - Automatic retry with exponential backoff
 * - Browser and Node.js compatibility
 *
 * @example Browser usage
 * ```typescript
 * const file = document.querySelector('input').files[0];
 * const upload = await client.upload(file, {
 *   repository: 'media',
 *   workspace: 'assets',
 *   path: '/videos/my-video.mp4',
 *   onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
 * });
 * const result = await upload.start();
 * ```
 *
 * @example Node.js usage
 * ```typescript
 * const upload = await client.uploadFile('/path/to/file.zip', {
 *   repository: 'data',
 *   workspace: 'backups',
 *   path: '/backup.zip'
 * });
 * await upload.start();
 * ```
 *
 * @example Pause and resume
 * ```typescript
 * const upload = await client.upload(file, options);
 * upload.start(); // Don't await - run in background
 *
 * // Later...
 * upload.pause();
 *
 * // Even later...
 * await upload.resume();
 * ```
 */

// Types
export type {
  UploadStatus,
  UploadOptions,
  UploadProgress,
  UploadResult,
  Upload,
  CreateUploadRequest,
  CreateUploadResponse,
  ChunkUploadResponse,
  UploadStatusResponse,
  CompleteUploadRequest,
  CompleteUploadResponse,
  RetryOptions,
  // Batch upload types
  BatchFileProgress,
  BatchProgress,
  BatchUploadResult,
  BatchUploadOptions,
  BatchUpload,
} from './types';

export {
  UploadError,
  UploadErrorCode,
  DEFAULT_CHUNK_SIZE,
  DEFAULT_MAX_RETRIES,
  DEFAULT_RETRY_BASE_DELAY,
  DEFAULT_RETRY_MAX_DELAY,
  DEFAULT_NODE_TYPE,
  DEFAULT_BRANCH,
  DEFAULT_BATCH_CONCURRENCY,
} from './types';

// File source
export type { FileSource } from './file-source';
export {
  BrowserFileSource,
  NodeFileSource,
  createFileSource,
  getContentType,
  calculateChunkCount,
  getChunkRange,
} from './file-source';

// Retry utilities
export type { RetryContext, RetryCallback } from './retry';
export {
  DEFAULT_RETRY_OPTIONS,
  calculateDelay,
  sleep,
  isRetryableError,
  withRetry,
  RetryBuilder,
  classifyError,
} from './retry';

// Uploader
export type { UploaderConfig } from './uploader';
export { Uploader, UploadManager } from './uploader';

// Batch uploader
export { BatchUploader } from './batch';
