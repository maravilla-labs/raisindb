/**
 * Main Upload implementation
 *
 * Provides resumable chunked uploads with progress tracking,
 * pause/resume, and automatic retry support.
 */

import type { AuthManager } from '../auth';
import type {
  Upload,
  UploadOptions,
  UploadProgress,
  UploadResult,
  UploadStatus,
  CreateUploadRequest,
  CreateUploadResponse,
  ChunkUploadResponse,
  CompleteUploadRequest,
  CompleteUploadResponse,
  UploadStatusResponse,
  RetryOptions,
  BatchUpload,
  BatchUploadOptions,
} from './types';
import {
  UploadError,
  UploadErrorCode,
  DEFAULT_CHUNK_SIZE,
  DEFAULT_MAX_RETRIES,
  DEFAULT_NODE_TYPE,
  DEFAULT_BRANCH,
} from './types';
import type { FileSource } from './file-source';
import { getContentType, getChunkRange, calculateChunkCount } from './file-source';
import { withRetry } from './retry';
import { BatchUploader } from './batch';

// ============================================================================
// Speed Tracker
// ============================================================================

/**
 * Track upload speed using a sliding window
 */
class SpeedTracker {
  private samples: Array<{ bytes: number; timestamp: number }> = [];
  private readonly windowSize: number;
  private readonly maxSamples: number;

  constructor(windowSizeMs = 5000, maxSamples = 20) {
    this.windowSize = windowSizeMs;
    this.maxSamples = maxSamples;
  }

  /**
   * Add a sample
   */
  addSample(bytes: number): void {
    const now = Date.now();
    this.samples.push({ bytes, timestamp: now });

    // Remove old samples
    const cutoff = now - this.windowSize;
    this.samples = this.samples.filter((s) => s.timestamp > cutoff);

    // Limit max samples
    if (this.samples.length > this.maxSamples) {
      this.samples = this.samples.slice(-this.maxSamples);
    }
  }

  /**
   * Get current speed in bytes/second
   */
  getSpeed(): number {
    if (this.samples.length < 2) {
      return 0;
    }

    const first = this.samples[0];
    const last = this.samples[this.samples.length - 1];
    const timeDelta = (last.timestamp - first.timestamp) / 1000; // seconds

    if (timeDelta === 0) {
      return 0;
    }

    const totalBytes = this.samples.reduce((sum, s) => sum + s.bytes, 0);
    return totalBytes / timeDelta;
  }

  /**
   * Calculate ETA in seconds
   */
  getEta(remainingBytes: number): number {
    const speed = this.getSpeed();
    if (speed === 0) {
      return 0;
    }
    return remainingBytes / speed;
  }

  /**
   * Reset tracker
   */
  reset(): void {
    this.samples = [];
  }
}

// ============================================================================
// Chunk Uploader (Browser vs Node.js)
// ============================================================================

/**
 * Upload a chunk using XHR (browser) for progress tracking
 */
async function uploadChunkXhr(
  url: string,
  chunk: ArrayBuffer,
  headers: Record<string, string>,
  onProgress?: (loaded: number) => void,
  signal?: AbortSignal
): Promise<Response> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();

    // Handle abort
    const onAbort = () => {
      xhr.abort();
      reject(UploadError.cancelledError());
    };

    if (signal) {
      if (signal.aborted) {
        reject(UploadError.cancelledError());
        return;
      }
      signal.addEventListener('abort', onAbort, { once: true });
    }

    // Track upload progress
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable && onProgress) {
        onProgress(event.loaded);
      }
    };

    // Handle completion
    xhr.onload = () => {
      if (signal) {
        signal.removeEventListener('abort', onAbort);
      }

      // Create a Response-like object
      const response = new Response(xhr.responseText, {
        status: xhr.status,
        statusText: xhr.statusText,
        headers: parseXhrHeaders(xhr.getAllResponseHeaders()),
      });

      resolve(response);
    };

    // Handle errors
    xhr.onerror = () => {
      if (signal) {
        signal.removeEventListener('abort', onAbort);
      }
      reject(UploadError.networkError('XHR network error'));
    };

    xhr.ontimeout = () => {
      if (signal) {
        signal.removeEventListener('abort', onAbort);
      }
      reject(UploadError.timeoutError('XHR timeout'));
    };

    xhr.onabort = () => {
      if (signal) {
        signal.removeEventListener('abort', onAbort);
      }
      reject(UploadError.cancelledError());
    };

    // Open and send
    xhr.open('PATCH', url);

    // Set headers
    for (const [key, value] of Object.entries(headers)) {
      xhr.setRequestHeader(key, value);
    }

    xhr.send(chunk);
  });
}

/**
 * Parse XHR response headers into Headers object
 */
function parseXhrHeaders(headerString: string): Headers {
  const headers = new Headers();
  const lines = headerString.trim().split(/[\r\n]+/);

  for (const line of lines) {
    const parts = line.split(': ');
    const key = parts.shift();
    const value = parts.join(': ');
    if (key) {
      headers.append(key, value);
    }
  }

  return headers;
}

/**
 * Upload a chunk using fetch (Node.js or fallback)
 */
async function uploadChunkFetch(
  url: string,
  chunk: ArrayBuffer,
  headers: Record<string, string>,
  signal?: AbortSignal,
  fetchFn: typeof fetch = fetch
): Promise<Response> {
  try {
    return await fetchFn(url, {
      method: 'PATCH',
      headers,
      body: chunk,
      signal,
    });
  } catch (error) {
    if (error instanceof Error) {
      if (error.name === 'AbortError') {
        throw UploadError.cancelledError();
      }
      throw UploadError.networkError(error.message, error);
    }
    throw error;
  }
}

/**
 * Detect if we're in a browser environment
 */
function isBrowser(): boolean {
  return typeof window !== 'undefined' && typeof XMLHttpRequest !== 'undefined';
}

// ============================================================================
// Uploader Implementation
// ============================================================================

/**
 * Configuration for creating an Uploader
 */
export interface UploaderConfig {
  /** Base URL for the API */
  baseUrl: string;
  /** Auth manager for authentication */
  authManager: AuthManager;
  /** Custom fetch implementation */
  fetchImpl?: typeof fetch;
  /** Request timeout in milliseconds */
  requestTimeout?: number;
}

/**
 * Main upload implementation
 */
export class Uploader implements Upload {
  readonly id: string;

  private readonly baseUrl: string;
  private readonly authManager: AuthManager;
  private readonly fetchImpl: typeof fetch;
  private readonly requestTimeout: number;

  private readonly fileSource: FileSource;
  private readonly options: Required<
    Omit<UploadOptions, 'onProgress' | 'signal' | 'metadata' | 'commitMessage' | 'commitActor'>
  > & Pick<UploadOptions, 'onProgress' | 'signal' | 'metadata' | 'commitMessage' | 'commitActor'>;

  private session: CreateUploadResponse | null = null;
  private status: UploadStatus = 'pending';
  private bytesUploaded = 0;
  private currentChunk = 0;
  private totalChunks = 0;
  private readonly speedTracker = new SpeedTracker();

  private pauseRequested = false;
  private abortController: AbortController | null = null;

  /**
   * Create a new Uploader
   */
  constructor(
    config: UploaderConfig,
    fileSource: FileSource,
    options: UploadOptions,
    uploadId?: string
  ) {
    this.baseUrl = config.baseUrl.replace(/\/$/, '');
    this.authManager = config.authManager;
    this.fetchImpl = config.fetchImpl ?? fetch;
    this.requestTimeout = config.requestTimeout ?? 30000;

    this.fileSource = fileSource;
    this.id = uploadId ?? generateUploadId();

    // Merge with defaults
    this.options = {
      repository: options.repository,
      branch: options.branch ?? DEFAULT_BRANCH,
      workspace: options.workspace,
      path: options.path,
      nodeType: options.nodeType ?? DEFAULT_NODE_TYPE,
      chunkSize: options.chunkSize ?? DEFAULT_CHUNK_SIZE,
      commitMessage: options.commitMessage,
      commitActor: options.commitActor,
      autoRetry: options.autoRetry ?? true,
      maxRetries: options.maxRetries ?? DEFAULT_MAX_RETRIES,
      onProgress: options.onProgress,
      signal: options.signal,
      metadata: options.metadata,
    };

    // Calculate total chunks
    this.totalChunks = calculateChunkCount(
      this.fileSource.size,
      this.options.chunkSize
    );
  }

  /**
   * Start the upload
   */
  async start(): Promise<UploadResult> {
    if (this.status === 'completed') {
      throw new UploadError(
        'Upload already completed',
        UploadErrorCode.INVALID_FILE,
        false
      );
    }

    if (this.status === 'uploading') {
      throw new UploadError(
        'Upload already in progress',
        UploadErrorCode.INVALID_FILE,
        false
      );
    }

    this.pauseRequested = false;
    this.abortController = new AbortController();

    // Link external abort signal
    if (this.options.signal) {
      this.options.signal.addEventListener('abort', () => {
        this.abortController?.abort();
      });
    }

    try {
      // Create session if not already created
      if (!this.session) {
        await this.createSession();
      }

      // Upload chunks
      this.status = 'uploading';
      this.emitProgress();

      await this.uploadChunks();

      // Complete the upload
      this.status = 'completing';
      this.emitProgress();

      const result = await this.completeUpload();

      this.status = 'completed';
      this.emitProgress();

      return result;
    } catch (error) {
      if (this.status !== 'paused' && this.status !== 'cancelled') {
        this.status = 'failed';
        this.emitProgress();
      }
      throw error;
    }
  }

  /**
   * Pause the upload
   */
  pause(): void {
    if (this.status !== 'uploading') {
      return;
    }

    this.pauseRequested = true;
    this.status = 'paused';
    this.emitProgress();
  }

  /**
   * Resume a paused upload
   */
  async resume(): Promise<UploadResult> {
    if (this.status !== 'paused') {
      throw new UploadError(
        'Upload is not paused',
        UploadErrorCode.INVALID_FILE,
        false
      );
    }

    // Refresh session status
    await this.refreshSessionStatus();

    // Continue uploading
    return this.start();
  }

  /**
   * Cancel the upload
   */
  async cancel(): Promise<void> {
    this.pauseRequested = true;
    this.abortController?.abort();
    this.status = 'cancelled';
    this.emitProgress();

    // Try to cancel server-side
    if (this.session) {
      try {
        await this.deleteSession();
      } catch {
        // Ignore errors during cancel
      }
    }
  }

  /**
   * Get current progress
   */
  getProgress(): UploadProgress {
    const bytesTotal = this.fileSource.size;
    const progress = bytesTotal > 0 ? this.bytesUploaded / bytesTotal : 0;
    const speed = this.speedTracker.getSpeed();
    const remainingBytes = bytesTotal - this.bytesUploaded;
    const eta = this.speedTracker.getEta(remainingBytes);

    return {
      bytesUploaded: this.bytesUploaded,
      bytesTotal,
      progress,
      speed,
      eta,
      currentChunk: this.currentChunk + 1, // 1-indexed for display
      totalChunks: this.totalChunks,
      status: this.status,
    };
  }

  /**
   * Check if upload is active
   */
  isActive(): boolean {
    return this.status === 'uploading' || this.status === 'completing';
  }

  // ============================================================================
  // Private Methods
  // ============================================================================

  /**
   * Create upload session
   */
  private async createSession(): Promise<void> {
    const contentType = getContentType(this.fileSource);

    const request: CreateUploadRequest = {
      repository: this.options.repository,
      branch: this.options.branch,
      workspace: this.options.workspace,
      path: this.options.path,
      filename: this.fileSource.name,
      file_size: this.fileSource.size,
      content_type: contentType,
      node_type: this.options.nodeType,
      chunk_size: this.options.chunkSize,
      metadata: this.options.metadata,
    };

    const response = await this.apiRequest<CreateUploadResponse>(
      '/api/uploads',
      {
        method: 'POST',
        body: request,
      }
    );

    this.session = response;
    this.totalChunks = response.total_chunks;
  }

  /**
   * Refresh session status from server
   */
  private async refreshSessionStatus(): Promise<void> {
    if (!this.session) {
      return;
    }

    const response = await this.apiRequest<UploadStatusResponse>(
      `/api/uploads/${this.session.upload_id}`,
      { method: 'GET' }
    );

    // Update local state based on server state
    this.bytesUploaded = response.bytes_received;
    this.currentChunk = response.chunks_completed;

    // Check for expiration
    const expiresAt = new Date(response.expires_at);
    if (expiresAt < new Date()) {
      throw UploadError.sessionExpiredError(this.session.upload_id);
    }
  }

  /**
   * Upload all remaining chunks
   */
  private async uploadChunks(): Promise<void> {
    const signal = this.abortController?.signal;

    while (this.currentChunk < this.totalChunks) {
      // Check for pause/cancel
      if (this.pauseRequested || signal?.aborted) {
        if (signal?.aborted) {
          throw UploadError.cancelledError();
        }
        return; // Paused
      }

      // Upload chunk with retry
      if (this.options.autoRetry) {
        const retryOptions: Partial<RetryOptions> = {
          maxRetries: this.options.maxRetries,
        };

        await withRetry(
          () => this.uploadChunk(this.currentChunk),
          retryOptions,
          signal
        );
      } else {
        await this.uploadChunk(this.currentChunk);
      }

      this.currentChunk++;
    }
  }

  /**
   * Upload a single chunk
   */
  private async uploadChunk(chunkIndex: number): Promise<void> {
    if (!this.session) {
      throw new UploadError(
        'No active session',
        UploadErrorCode.SESSION_NOT_FOUND,
        false
      );
    }

    const signal = this.abortController?.signal;
    const { start, end } = getChunkRange(
      chunkIndex,
      this.options.chunkSize,
      this.fileSource.size
    );

    // Read chunk data
    const chunk = await this.fileSource.slice(start, end);

    // Build headers
    const headers: Record<string, string> = {
      'Content-Type': 'application/octet-stream',
      'Content-Range': `bytes ${start}-${end - 1}/${this.fileSource.size}`,
    };

    // Add auth header
    const token = this.authManager.getAccessToken();
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }

    const url = `${this.baseUrl}/api/uploads/${this.session.upload_id}`;

    // Upload using XHR (browser) or fetch (Node.js)
    let response: Response;

    const chunkStartBytes = this.bytesUploaded;

    if (isBrowser()) {
      response = await uploadChunkXhr(
        url,
        chunk,
        headers,
        (loaded) => {
          // Update bytes uploaded with partial progress
          this.bytesUploaded = chunkStartBytes + loaded;
          this.speedTracker.addSample(loaded);
          this.emitProgress();
        },
        signal
      );
    } else {
      response = await uploadChunkFetch(url, chunk, headers, signal, this.fetchImpl);
    }

    // Handle response
    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw UploadError.fromResponse(response.status, body);
    }

    const result: ChunkUploadResponse = await response.json();

    // Update state
    this.bytesUploaded = result.bytes_received;
    this.speedTracker.addSample(end - start);
    this.emitProgress();
  }

  /**
   * Complete the upload
   */
  private async completeUpload(): Promise<UploadResult> {
    if (!this.session) {
      throw new UploadError(
        'No active session',
        UploadErrorCode.SESSION_NOT_FOUND,
        false
      );
    }

    const request: CompleteUploadRequest = {
      commit_message: this.options.commitMessage,
      commit_actor: this.options.commitActor,
    };

    const response = await this.apiRequest<CompleteUploadResponse>(
      `/api/uploads/${this.session.upload_id}/complete`,
      {
        method: 'POST',
        body: request,
      }
    );

    return {
      node: response.node,
      revision: response.revision,
      storageKey: response.storage_key,
    };
  }

  /**
   * Delete upload session
   */
  private async deleteSession(): Promise<void> {
    if (!this.session) {
      return;
    }

    await this.apiRequest<void>(
      `/api/uploads/${this.session.upload_id}`,
      { method: 'DELETE' }
    );
  }

  /**
   * Make an API request
   */
  private async apiRequest<T>(
    path: string,
    options: {
      method: string;
      body?: unknown;
    }
  ): Promise<T> {
    const url = `${this.baseUrl}${path}`;

    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };

    // Add auth header
    const token = this.authManager.getAccessToken();
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }

    // Create abort controller for timeout
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.requestTimeout);

    try {
      const response = await this.fetchImpl(url, {
        method: options.method,
        headers,
        body: options.body ? JSON.stringify(options.body) : undefined,
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw UploadError.fromResponse(response.status, body);
      }

      // Handle empty responses
      const text = await response.text();
      if (!text) {
        return undefined as T;
      }

      return JSON.parse(text) as T;
    } catch (error) {
      clearTimeout(timeoutId);

      if (error instanceof Error && error.name === 'AbortError') {
        throw UploadError.timeoutError(`Request timeout after ${this.requestTimeout}ms`);
      }

      throw error;
    }
  }

  /**
   * Emit progress update
   */
  private emitProgress(): void {
    if (this.options.onProgress) {
      this.options.onProgress(this.getProgress());
    }
  }
}

// ============================================================================
// Helpers
// ============================================================================

/**
 * Generate a unique upload ID
 */
function generateUploadId(): string {
  // Use crypto.randomUUID if available
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }

  // Fallback to manual UUID generation
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

// ============================================================================
// Upload Manager
// ============================================================================

/**
 * Manager for creating and tracking uploads
 */
export class UploadManager {
  private readonly config: UploaderConfig;
  private readonly uploads = new Map<string, Uploader>();

  /**
   * Create an upload manager
   */
  constructor(config: UploaderConfig) {
    this.config = config;
  }

  /**
   * Create a new upload
   */
  createUpload(fileSource: FileSource, options: UploadOptions): Upload {
    const uploader = new Uploader(this.config, fileSource, options);
    this.uploads.set(uploader.id, uploader);
    return uploader;
  }

  /**
   * Get an existing upload by ID
   */
  getUpload(id: string): Upload | undefined {
    return this.uploads.get(id);
  }

  /**
   * Get all active uploads
   */
  getActiveUploads(): Upload[] {
    return Array.from(this.uploads.values()).filter((u) => u.isActive());
  }

  /**
   * Cancel all active uploads
   */
  async cancelAll(): Promise<void> {
    const active = this.getActiveUploads();
    await Promise.all(active.map((u) => u.cancel()));
  }

  /**
   * Remove completed/cancelled uploads from tracking
   */
  cleanup(): void {
    for (const [id, upload] of this.uploads) {
      const progress = upload.getProgress();
      if (
        progress.status === 'completed' ||
        progress.status === 'cancelled' ||
        progress.status === 'failed'
      ) {
        this.uploads.delete(id);
      }
    }
  }

  /**
   * Create a batch upload for multiple files
   *
   * @param files - Array of FileSource objects to upload
   * @param options - Batch upload options
   * @returns BatchUpload controller
   *
   * @example
   * ```typescript
   * const batch = manager.createBatchUpload(fileSources, {
   *   repository: 'media',
   *   workspace: 'assets',
   *   basePath: '/uploads',
   *   concurrency: 3,
   *   onProgress: (p) => console.log(`${p.filesCompleted}/${p.filesTotal}`)
   * });
   * const result = await batch.start();
   * ```
   */
  createBatchUpload(files: FileSource[], options: BatchUploadOptions): BatchUpload {
    return new BatchUploader(this.config, files, options);
  }
}
