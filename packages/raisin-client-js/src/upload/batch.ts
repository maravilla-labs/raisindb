/**
 * Batch upload implementation for multiple files
 *
 * Provides concurrent upload of multiple files with:
 * - Configurable concurrency limit
 * - Aggregate and per-file progress tracking
 * - Pause/resume/cancel support
 * - Error handling with continue-on-error option
 */

import type {
  BatchUpload,
  BatchUploadOptions,
  BatchUploadResult,
  BatchProgress,
  BatchFileProgress,
  UploadResult,
  UploadStatus,
} from './types';
import {
  DEFAULT_BATCH_CONCURRENCY,
  DEFAULT_BRANCH,
  DEFAULT_NODE_TYPE,
  DEFAULT_CHUNK_SIZE,
  DEFAULT_MAX_RETRIES,
} from './types';
import type { FileSource } from './file-source';
import type { UploaderConfig } from './uploader';
import { Uploader } from './uploader';

// ============================================================================
// Speed Tracker for Batch
// ============================================================================

/**
 * Track aggregate upload speed using a sliding window
 */
class BatchSpeedTracker {
  private samples: Array<{ bytes: number; timestamp: number }> = [];
  private readonly windowSize: number;
  private readonly maxSamples: number;

  constructor(windowSizeMs = 5000, maxSamples = 50) {
    this.windowSize = windowSizeMs;
    this.maxSamples = maxSamples;
  }

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

  getSpeed(): number {
    if (this.samples.length < 2) {
      return 0;
    }

    const first = this.samples[0];
    const last = this.samples[this.samples.length - 1];
    const timeDelta = (last.timestamp - first.timestamp) / 1000;

    if (timeDelta === 0) {
      return 0;
    }

    const totalBytes = this.samples.reduce((sum, s) => sum + s.bytes, 0);
    return totalBytes / timeDelta;
  }

  getEta(remainingBytes: number): number {
    const speed = this.getSpeed();
    if (speed === 0) {
      return 0;
    }
    return remainingBytes / speed;
  }

  reset(): void {
    this.samples = [];
  }
}

// ============================================================================
// File State
// ============================================================================

interface FileState {
  file: FileSource;
  status: UploadStatus;
  bytesUploaded: number;
  uploader: Uploader | null;
  result: UploadResult | null;
  error: Error | null;
}

// ============================================================================
// Batch Uploader
// ============================================================================

/**
 * Manages concurrent upload of multiple files
 */
export class BatchUploader implements BatchUpload {
  private readonly config: UploaderConfig;
  private readonly files: FileSource[];
  private readonly options: Required<
    Omit<
      BatchUploadOptions,
      'onProgress' | 'onFileComplete' | 'onFileError' | 'signal' | 'pathResolver' | 'metadata' | 'commitMessage' | 'commitActor'
    >
  > &
    Pick<BatchUploadOptions, 'onProgress' | 'onFileComplete' | 'onFileError' | 'signal' | 'pathResolver' | 'metadata' | 'commitMessage' | 'commitActor'>;

  private readonly fileStates: Map<string, FileState> = new Map();
  private readonly queue: string[] = [];
  private readonly speedTracker = new BatchSpeedTracker();

  private activeWorkers = 0;
  private paused = false;
  private cancelled = false;
  private started = false;
  private resolveWhenResumed: (() => void) | null = null;
  private lastBytesUploaded = 0;

  /**
   * Create a batch uploader
   */
  constructor(
    config: UploaderConfig,
    files: FileSource[],
    options: BatchUploadOptions
  ) {
    this.config = config;
    this.files = files;

    // Merge with defaults
    this.options = {
      repository: options.repository,
      branch: options.branch ?? DEFAULT_BRANCH,
      workspace: options.workspace,
      basePath: options.basePath ?? '',
      pathResolver: options.pathResolver,
      nodeType: options.nodeType ?? DEFAULT_NODE_TYPE,
      metadata: options.metadata,
      chunkSize: options.chunkSize ?? DEFAULT_CHUNK_SIZE,
      concurrency: options.concurrency ?? DEFAULT_BATCH_CONCURRENCY,
      onProgress: options.onProgress,
      onFileComplete: options.onFileComplete,
      onFileError: options.onFileError,
      signal: options.signal,
      commitMessage: options.commitMessage,
      commitActor: options.commitActor,
      autoRetry: options.autoRetry ?? true,
      maxRetries: options.maxRetries ?? DEFAULT_MAX_RETRIES,
      continueOnError: options.continueOnError ?? true,
    };

    // Initialize file states
    for (const file of files) {
      this.fileStates.set(file.name, {
        file,
        status: 'pending',
        bytesUploaded: 0,
        uploader: null,
        result: null,
        error: null,
      });
      this.queue.push(file.name);
    }
  }

  /**
   * Start all uploads
   */
  async start(): Promise<BatchUploadResult> {
    if (this.started && !this.paused) {
      throw new Error('Batch upload already started');
    }

    this.started = true;
    this.paused = false;
    this.cancelled = false;

    // Link external abort signal
    if (this.options.signal) {
      this.options.signal.addEventListener('abort', () => {
        this.cancel();
      });
    }

    // Start workers
    const concurrency = Math.min(this.options.concurrency, this.queue.length);
    const workers = Array(concurrency)
      .fill(null)
      .map(() => this.worker());

    await Promise.all(workers);

    return this.getResult();
  }

  /**
   * Pause all active uploads
   */
  pause(): void {
    if (!this.started || this.paused || this.cancelled) {
      return;
    }

    this.paused = true;

    // Pause all active uploaders
    for (const state of this.fileStates.values()) {
      if (state.uploader && state.status === 'uploading') {
        state.uploader.pause();
        state.status = 'paused';
      }
    }

    this.emitProgress();
  }

  /**
   * Resume paused uploads
   */
  async resume(): Promise<BatchUploadResult> {
    if (!this.paused) {
      throw new Error('Batch upload is not paused');
    }

    this.paused = false;

    // Resume all paused uploaders
    for (const state of this.fileStates.values()) {
      if (state.status === 'paused') {
        state.status = 'uploading';
      }
    }

    // Wake up waiting workers
    if (this.resolveWhenResumed) {
      this.resolveWhenResumed();
      this.resolveWhenResumed = null;
    }

    this.emitProgress();

    // Continue with remaining uploads
    return this.start();
  }

  /**
   * Cancel all uploads
   */
  async cancel(): Promise<void> {
    this.cancelled = true;
    this.paused = false;

    // Cancel all active uploaders
    const cancelPromises: Promise<void>[] = [];
    for (const state of this.fileStates.values()) {
      if (state.uploader && (state.status === 'uploading' || state.status === 'paused')) {
        cancelPromises.push(state.uploader.cancel());
        state.status = 'cancelled';
      }
    }

    // Mark pending files as cancelled
    for (const state of this.fileStates.values()) {
      if (state.status === 'pending') {
        state.status = 'cancelled';
      }
    }

    // Wake up waiting workers
    if (this.resolveWhenResumed) {
      this.resolveWhenResumed();
      this.resolveWhenResumed = null;
    }

    await Promise.all(cancelPromises);

    this.emitProgress();
  }

  /**
   * Get current aggregate progress
   */
  getProgress(): BatchProgress {
    const fileProgresses: BatchFileProgress[] = [];

    let bytesUploaded = 0;
    let bytesTotal = 0;
    let filesCompleted = 0;
    let filesFailed = 0;
    let filesInProgress = 0;
    let filesPending = 0;

    for (const state of this.fileStates.values()) {
      const fileProgress: BatchFileProgress = {
        file: state.file.name,
        status: state.status,
        bytesUploaded: state.bytesUploaded,
        bytesTotal: state.file.size,
        progress: state.file.size > 0 ? state.bytesUploaded / state.file.size : 0,
        error: state.error ?? undefined,
      };
      fileProgresses.push(fileProgress);

      bytesUploaded += state.bytesUploaded;
      bytesTotal += state.file.size;

      switch (state.status) {
        case 'completed':
          filesCompleted++;
          break;
        case 'failed':
          filesFailed++;
          break;
        case 'uploading':
        case 'completing':
          filesInProgress++;
          break;
        case 'pending':
        case 'paused':
          filesPending++;
          break;
      }
    }

    const progress = bytesTotal > 0 ? bytesUploaded / bytesTotal : 0;
    const speed = this.speedTracker.getSpeed();
    const remainingBytes = bytesTotal - bytesUploaded;
    const eta = this.speedTracker.getEta(remainingBytes);

    return {
      filesTotal: this.files.length,
      filesCompleted,
      filesFailed,
      filesInProgress,
      filesPending,
      bytesUploaded,
      bytesTotal,
      progress,
      speed,
      eta,
      files: fileProgresses,
    };
  }

  /**
   * Check if any uploads are active
   */
  isActive(): boolean {
    return this.started && !this.paused && !this.cancelled && this.activeWorkers > 0;
  }

  // ============================================================================
  // Private Methods
  // ============================================================================

  /**
   * Worker that processes files from the queue
   */
  private async worker(): Promise<void> {
    this.activeWorkers++;

    try {
      while (this.queue.length > 0 && !this.cancelled) {
        // Wait if paused
        if (this.paused) {
          await this.waitForResume();
          if (this.cancelled) break;
        }

        // Get next file from queue
        const fileName = this.queue.shift();
        if (!fileName) break;

        const state = this.fileStates.get(fileName);
        if (!state) continue;

        // Upload the file
        await this.uploadFile(state);
      }
    } finally {
      this.activeWorkers--;
    }
  }

  /**
   * Wait until resume() is called
   */
  private async waitForResume(): Promise<void> {
    return new Promise((resolve) => {
      this.resolveWhenResumed = resolve;
    });
  }

  /**
   * Upload a single file
   */
  private async uploadFile(state: FileState): Promise<void> {
    const path = this.resolvePath(state.file);

    // Create uploader
    const uploader = new Uploader(
      this.config,
      state.file,
      {
        repository: this.options.repository,
        branch: this.options.branch,
        workspace: this.options.workspace,
        path,
        nodeType: this.options.nodeType,
        metadata: this.options.metadata,
        chunkSize: this.options.chunkSize,
        commitMessage: this.options.commitMessage,
        commitActor: this.options.commitActor,
        autoRetry: this.options.autoRetry,
        maxRetries: this.options.maxRetries,
        onProgress: (progress) => {
          state.bytesUploaded = progress.bytesUploaded;
          state.status = progress.status;
          this.updateSpeedTracker();
          this.emitProgress();
        },
      }
    );

    state.uploader = uploader;
    state.status = 'uploading';

    try {
      const result = await uploader.start();
      state.status = 'completed';
      state.result = result;
      state.bytesUploaded = state.file.size;

      // Notify file complete
      this.options.onFileComplete?.(state.file.name, result);
    } catch (error) {
      state.status = 'failed';
      state.error = error instanceof Error ? error : new Error(String(error));

      // Notify file error
      this.options.onFileError?.(state.file.name, state.error);

      // Stop if not continuing on error
      if (!this.options.continueOnError) {
        this.cancelled = true;
      }
    } finally {
      this.emitProgress();
    }
  }

  /**
   * Resolve the path for a file
   */
  private resolvePath(file: FileSource): string {
    if (this.options.pathResolver) {
      return this.options.pathResolver({ name: file.name, size: file.size });
    }

    // Default: basePath/filename
    const basePath = this.options.basePath.replace(/\/$/, '');
    return basePath ? `${basePath}/${file.name}` : `/${file.name}`;
  }

  /**
   * Update speed tracker with delta bytes
   */
  private updateSpeedTracker(): void {
    const progress = this.getProgress();
    const delta = progress.bytesUploaded - this.lastBytesUploaded;
    if (delta > 0) {
      this.speedTracker.addSample(delta);
      this.lastBytesUploaded = progress.bytesUploaded;
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

  /**
   * Get final result
   */
  private getResult(): BatchUploadResult {
    const successful: Array<{ file: string; result: UploadResult }> = [];
    const failed: Array<{ file: string; error: Error }> = [];

    for (const state of this.fileStates.values()) {
      if (state.status === 'completed' && state.result) {
        successful.push({ file: state.file.name, result: state.result });
      } else if (state.status === 'failed' && state.error) {
        failed.push({ file: state.file.name, error: state.error });
      } else if (state.status === 'cancelled') {
        failed.push({
          file: state.file.name,
          error: new Error('Upload cancelled'),
        });
      }
    }

    return { successful, failed };
  }
}
