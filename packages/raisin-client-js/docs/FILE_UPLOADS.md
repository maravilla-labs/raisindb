# File Uploads

Resumable chunked uploads with progress tracking, pause/resume, and automatic retry.

## Features

- **Resumable** - uploads continue from where they left off
- **Chunked** - large files split into 10MB chunks
- **Progress tracking** - real-time progress, speed, and ETA
- **Pause/Resume** - suspend and continue uploads
- **Cancel** - abort uploads at any time
- **Auto-retry** - automatic retry with exponential backoff
- **Batch uploads** - upload multiple files with concurrency control

---

## Single File Upload

### Basic Upload

```typescript
const client = new RaisinClient('raisin://localhost:8080/sys/default');
await client.connect();

const file = document.querySelector('input[type="file"]').files[0];

const upload = await client.upload(file, {
  repository: 'media',
  workspace: 'assets',
  path: '/videos/my-video.mp4'
});

const result = await upload.start();
console.log('Uploaded:', result.node.id);
```

### With Progress Tracking

```typescript
const upload = await client.upload(file, {
  repository: 'media',
  workspace: 'assets',
  path: '/videos/my-video.mp4',
  onProgress: (progress) => {
    console.log(`Progress: ${Math.round(progress.progress * 100)}%`);
    console.log(`Uploaded: ${progress.bytesUploaded} / ${progress.bytesTotal} bytes`);
    console.log(`Speed: ${(progress.speed / 1024 / 1024).toFixed(2)} MB/s`);
    console.log(`ETA: ${Math.round(progress.eta)} seconds`);
    console.log(`Chunk: ${progress.currentChunk} / ${progress.totalChunks}`);
    console.log(`Status: ${progress.status}`);
  }
});

await upload.start();
```

### Progress Object

```typescript
interface UploadProgress {
  bytesUploaded: number;    // Bytes uploaded so far
  bytesTotal: number;       // Total file size
  progress: number;         // 0-1 fraction
  speed: number;            // Bytes per second
  eta: number;              // Estimated seconds remaining
  currentChunk: number;     // Current chunk (1-indexed)
  totalChunks: number;      // Total chunks
  status: UploadStatus;     // Current status
}

type UploadStatus =
  | 'pending'      // Not started
  | 'uploading'    // In progress
  | 'paused'       // Paused by user
  | 'completing'   // Finalizing upload
  | 'completed'    // Done
  | 'failed'       // Error occurred
  | 'cancelled';   // Cancelled by user
```

---

## Pause and Resume

```typescript
const upload = await client.upload(file, {
  repository: 'media',
  workspace: 'assets',
  path: '/video.mp4',
  onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
});

// Start upload (don't await - runs in background)
const uploadPromise = upload.start();

// Later... pause the upload
upload.pause();
console.log('Paused at:', upload.getProgress().progress * 100, '%');

// Even later... resume
const result = await upload.resume();
console.log('Completed:', result.node.id);
```

---

## Cancel Upload

```typescript
const upload = await client.upload(file, {
  repository: 'media',
  workspace: 'assets',
  path: '/video.mp4'
});

// Start in background
const uploadPromise = upload.start();

// Cancel it
await upload.cancel();
console.log('Upload cancelled');

// uploadPromise will reject with UploadError (code: CANCELLED)
```

---

## Upload with AbortSignal

```typescript
const controller = new AbortController();

const upload = await client.upload(file, {
  repository: 'media',
  workspace: 'assets',
  path: '/video.mp4',
  signal: controller.signal
});

// Start upload
const uploadPromise = upload.start();

// Abort from outside
controller.abort();
```

---

## Retry Options

```typescript
const upload = await client.upload(file, {
  repository: 'media',
  workspace: 'assets',
  path: '/video.mp4',
  autoRetry: true,      // Enable auto-retry (default: true)
  maxRetries: 5         // Max retry attempts (default: 3)
});
```

---

## Upload Options

```typescript
interface UploadOptions {
  repository: string;           // Repository name
  workspace: string;            // Workspace name
  path: string;                 // Node path for uploaded file
  branch?: string;              // Branch (default: "main")
  nodeType?: string;            // Node type (default: "raisin:Asset")
  metadata?: Record<string, unknown>;  // Additional node properties
  chunkSize?: number;           // Chunk size in bytes (default: 10MB)
  onProgress?: (progress: UploadProgress) => void;
  signal?: AbortSignal;         // For external cancellation
  commitMessage?: string;       // Git-style commit message
  commitActor?: string;         // User identifier for commit
  autoRetry?: boolean;          // Auto-retry on failure (default: true)
  maxRetries?: number;          // Max retries (default: 3)
}
```

---

## Batch Upload (Multiple Files)

### Basic Batch Upload

```typescript
const files = document.querySelector('input[type="file"]').files;

const batch = await client.uploadFiles(files, {
  repository: 'media',
  workspace: 'assets',
  basePath: '/uploads'    // Files go to /uploads/filename
});

const result = await batch.start();
console.log('Successful:', result.successful.length);
console.log('Failed:', result.failed.length);
```

### With Progress Tracking

```typescript
const batch = await client.uploadFiles(files, {
  repository: 'media',
  workspace: 'assets',
  basePath: '/uploads',
  concurrency: 3,         // Max parallel uploads

  // Aggregate progress
  onProgress: (progress) => {
    console.log('--- Overall ---');
    console.log(`Files: ${progress.filesCompleted}/${progress.filesTotal}`);
    console.log(`Progress: ${Math.round(progress.progress * 100)}%`);
    console.log(`Speed: ${(progress.speed / 1024 / 1024).toFixed(2)} MB/s`);
    console.log(`ETA: ${Math.round(progress.eta)}s`);

    console.log('--- Per File ---');
    progress.files.forEach(file => {
      const pct = Math.round(file.progress * 100);
      console.log(`  ${file.file}: ${pct}% [${file.status}]`);
    });
  }
});

await batch.start();
```

### Per-File Callbacks

```typescript
const batch = await client.uploadFiles(files, {
  repository: 'media',
  workspace: 'assets',
  basePath: '/uploads',

  onProgress: (p) => {
    console.log(`${p.filesCompleted}/${p.filesTotal} files complete`);
  },

  onFileComplete: (filename, result) => {
    console.log(`Uploaded: ${filename} -> ${result.node.id}`);
  },

  onFileError: (filename, error) => {
    console.error(`Failed: ${filename} - ${error.message}`);
  }
});

await batch.start();
```

### Batch Progress Object

```typescript
interface BatchProgress {
  filesTotal: number;       // Total files in batch
  filesCompleted: number;   // Successfully uploaded
  filesFailed: number;      // Failed uploads
  filesInProgress: number;  // Currently uploading
  filesPending: number;     // Waiting in queue
  bytesUploaded: number;    // Total bytes uploaded
  bytesTotal: number;       // Total bytes across all files
  progress: number;         // 0-1 overall progress
  speed: number;            // Aggregate bytes/second
  eta: number;              // Estimated seconds remaining
  files: BatchFileProgress[];  // Per-file details
}

interface BatchFileProgress {
  file: string;             // Filename
  status: UploadStatus;     // Current status
  bytesUploaded: number;
  bytesTotal: number;
  progress: number;         // 0-1
  error?: Error;            // Error if failed
}
```

---

## Batch Pause/Resume/Cancel

```typescript
const batch = await client.uploadFiles(files, {
  repository: 'media',
  workspace: 'assets',
  basePath: '/uploads'
});

// Start in background
const batchPromise = batch.start();

// Pause all uploads
batch.pause();

// Resume all uploads
const result = await batch.resume();

// Or cancel all
await batch.cancel();
```

---

## Batch Options

```typescript
interface BatchUploadOptions {
  repository: string;
  workspace: string;
  basePath?: string;            // Base path (files go to basePath/filename)
  pathResolver?: (file: { name: string; size: number }) => string;  // Custom paths
  branch?: string;
  nodeType?: string;
  metadata?: Record<string, unknown>;
  chunkSize?: number;
  concurrency?: number;         // Max parallel uploads (default: 3)
  onProgress?: (progress: BatchProgress) => void;
  onFileComplete?: (file: string, result: UploadResult) => void;
  onFileError?: (file: string, error: Error) => void;
  signal?: AbortSignal;
  commitMessage?: string;
  commitActor?: string;
  autoRetry?: boolean;
  maxRetries?: number;
  continueOnError?: boolean;    // Continue if one file fails (default: true)
}
```

### Custom Path Resolver

```typescript
const batch = await client.uploadFiles(files, {
  repository: 'media',
  workspace: 'assets',
  pathResolver: (file) => {
    const date = new Date().toISOString().split('T')[0];
    return `/uploads/${date}/${file.name}`;
  }
});
```

---

## Workspace Shorthand

When working within a specific workspace:

```typescript
const ws = client.database('media').workspace('assets');

// Single file
const upload = await ws.upload(file, '/video.mp4', {
  onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
});
await upload.start();

// Batch
const batch = await ws.uploadFiles(files, '/uploads', {
  concurrency: 3,
  onProgress: (p) => console.log(`${p.filesCompleted}/${p.filesTotal}`)
});
await batch.start();
```

---

## Managing Active Uploads

```typescript
// Get a specific upload by ID
const upload = client.getUpload('upload-id-123');

// Get all active uploads
const active = client.getActiveUploads();
console.log(`${active.length} uploads in progress`);

// Cancel all uploads
await client.cancelAllUploads();
```

---

## Error Handling

```typescript
import { UploadError, UploadErrorCode } from '@raisindb/client';

try {
  const upload = await client.upload(file, options);
  await upload.start();
} catch (error) {
  if (error instanceof UploadError) {
    switch (error.code) {
      case UploadErrorCode.CANCELLED:
        console.log('Upload was cancelled');
        break;
      case UploadErrorCode.NETWORK_ERROR:
        console.log('Network error - will retry');
        break;
      case UploadErrorCode.SESSION_EXPIRED:
        console.log('Session expired - restart upload');
        break;
      case UploadErrorCode.FILE_TOO_LARGE:
        console.log('File exceeds size limit');
        break;
      default:
        console.error('Upload failed:', error.message);
    }
  }
}
```

### Error Codes

```typescript
enum UploadErrorCode {
  INVALID_FILE         // Empty or invalid file
  FILE_TOO_LARGE       // Exceeds size limit
  SESSION_EXPIRED      // Upload session timed out
  SESSION_NOT_FOUND    // Session doesn't exist
  CHUNK_OFFSET_MISMATCH // Chunk position error
  CHECKSUM_MISMATCH    // Data integrity error
  STORAGE_ERROR        // Backend storage error
  SERVER_ERROR         // Server-side error
  NETWORK_ERROR        // Network connectivity (retryable)
  TIMEOUT              // Request timeout (retryable)
  CANCELLED            // User cancelled
}
```

---

## Node.js Usage

```typescript
// Upload from file path
const upload = await client.uploadFile('/path/to/file.zip', {
  repository: 'data',
  workspace: 'backups',
  path: '/backup.zip',
  onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
});

await upload.start();
```

---

## Upload Result

```typescript
interface UploadResult {
  node: Node;           // The created node
  revision?: string;    // Commit/revision ID
  storageKey: string;   // Storage key for binary content
}
```

---

## Constants

```typescript
import {
  DEFAULT_CHUNK_SIZE,        // 10 MB
  DEFAULT_MAX_RETRIES,       // 3
  DEFAULT_BATCH_CONCURRENCY, // 3
  DEFAULT_NODE_TYPE,         // 'raisin:Asset'
  DEFAULT_BRANCH             // 'main'
} from '@raisindb/client';
```
