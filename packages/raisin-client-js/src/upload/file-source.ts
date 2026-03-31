/**
 * File source abstraction for Browser and Node.js environments
 *
 * Provides a unified interface for reading file data in chunks,
 * supporting both browser File/Blob APIs and Node.js filesystem.
 */

// ============================================================================
// FileSource Interface
// ============================================================================

/**
 * Abstract interface for reading file data in chunks
 */
export interface FileSource {
  /** Total size of the file in bytes */
  readonly size: number;
  /** Name of the file */
  readonly name: string;
  /** MIME type of the file (optional) */
  readonly type?: string;
  /**
   * Read a slice of the file
   *
   * @param start - Start byte offset (inclusive)
   * @param end - End byte offset (exclusive)
   * @returns ArrayBuffer containing the slice data
   */
  slice(start: number, end: number): Promise<ArrayBuffer>;
}

// ============================================================================
// Browser Implementation
// ============================================================================

/**
 * FileSource implementation for browser File/Blob objects
 */
export class BrowserFileSource implements FileSource {
  private readonly _file: File | Blob;

  /**
   * Create a browser file source
   *
   * @param file - File or Blob object
   */
  constructor(file: File | Blob) {
    this._file = file;
  }

  /**
   * Total size in bytes
   */
  get size(): number {
    return this._file.size;
  }

  /**
   * File name (or "blob" for Blob objects)
   */
  get name(): string {
    return 'name' in this._file ? this._file.name : 'blob';
  }

  /**
   * MIME type
   */
  get type(): string | undefined {
    return this._file.type || undefined;
  }

  /**
   * Read a slice of the file
   */
  async slice(start: number, end: number): Promise<ArrayBuffer> {
    const blob = this._file.slice(start, end);
    return blob.arrayBuffer();
  }
}

// ============================================================================
// Node.js Implementation
// ============================================================================

/**
 * FileSource implementation for Node.js file paths
 *
 * Uses lazy initialization to avoid blocking on construction.
 * The fs module is dynamically imported to support browser bundling.
 */
export class NodeFileSource implements FileSource {
  private _size = 0;
  private readonly _name: string;
  private readonly _filePath: string;
  private _initialized = false;
  private _fsPromises: typeof import('fs/promises') | null = null;

  /**
   * Create a Node.js file source
   *
   * @param filePath - Path to the file
   * @param fsPromises - Optional fs/promises module (for testing)
   */
  constructor(
    filePath: string,
    fsPromises?: typeof import('fs/promises')
  ) {
    this._filePath = filePath;
    this._fsPromises = fsPromises ?? null;
    // Extract filename from path
    const parts = filePath.split(/[/\\]/);
    this._name = parts[parts.length - 1] || 'file';
  }

  /**
   * Initialize the file source (load fs module and get file stats)
   */
  private async init(): Promise<void> {
    if (this._initialized) {
      return;
    }

    // Dynamically import fs/promises if not provided
    if (!this._fsPromises) {
      // Use dynamic import for Node.js fs module
      this._fsPromises = await import('fs/promises');
    }

    // Get file stats to determine size
    const stats = await this._fsPromises.stat(this._filePath);
    this._size = stats.size;
    this._initialized = true;
  }

  /**
   * Total size in bytes
   *
   * Note: Returns 0 until initialized via slice() call
   */
  get size(): number {
    return this._size;
  }

  /**
   * File name
   */
  get name(): string {
    return this._name;
  }

  /**
   * MIME type (not available for Node.js files)
   */
  get type(): string | undefined {
    return undefined;
  }

  /**
   * Read a slice of the file
   */
  async slice(start: number, end: number): Promise<ArrayBuffer> {
    await this.init();

    // Open file for reading
    const fd = await this._fsPromises!.open(this._filePath, 'r');

    try {
      const length = end - start;
      const buffer = Buffer.alloc(length);
      await fd.read(buffer, 0, length, start);

      // Convert Buffer to ArrayBuffer
      return buffer.buffer.slice(
        buffer.byteOffset,
        buffer.byteOffset + buffer.byteLength
      );
    } finally {
      await fd.close();
    }
  }

  /**
   * Ensure the file source is initialized
   *
   * Call this before accessing size to ensure it's populated.
   */
  async ensureInitialized(): Promise<void> {
    await this.init();
  }
}

// ============================================================================
// Factory Function
// ============================================================================

/**
 * Detect the runtime environment
 */
function isNodeEnvironment(): boolean {
  return (
    typeof process !== 'undefined' &&
    process.versions != null &&
    process.versions.node != null
  );
}

/**
 * Create a FileSource from various inputs
 *
 * Automatically detects the input type and environment:
 * - File/Blob objects use BrowserFileSource
 * - String paths use NodeFileSource (Node.js only)
 *
 * @param input - File, Blob, or file path string
 * @returns Promise resolving to a FileSource
 * @throws Error if input type is not supported in the current environment
 */
export async function createFileSource(
  input: File | Blob | string
): Promise<FileSource> {
  // Handle string paths (Node.js file path)
  if (typeof input === 'string') {
    if (!isNodeEnvironment()) {
      throw new Error(
        'File path strings are only supported in Node.js. ' +
        'In the browser, use a File or Blob object.'
      );
    }

    const source = new NodeFileSource(input);
    // Force initialization to populate size
    await source.ensureInitialized();
    return source;
  }

  // Handle File/Blob objects
  if (input instanceof Blob) {
    return new BrowserFileSource(input);
  }

  throw new Error(
    'Unsupported input type. Expected File, Blob, or file path string.'
  );
}

// ============================================================================
// Utility Functions
// ============================================================================

/**
 * Get the content type for a file
 *
 * @param source - FileSource or filename
 * @returns MIME type or undefined
 */
export function getContentType(source: FileSource | string): string | undefined {
  if (typeof source === 'string') {
    return getMimeTypeFromExtension(source);
  }

  if (source.type) {
    return source.type;
  }

  return getMimeTypeFromExtension(source.name);
}

/**
 * Get MIME type from file extension
 */
function getMimeTypeFromExtension(filename: string): string | undefined {
  const ext = filename.split('.').pop()?.toLowerCase();

  if (!ext) {
    return undefined;
  }

  const mimeTypes: Record<string, string> = {
    // Images
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    png: 'image/png',
    gif: 'image/gif',
    webp: 'image/webp',
    svg: 'image/svg+xml',
    ico: 'image/x-icon',
    bmp: 'image/bmp',
    tiff: 'image/tiff',
    tif: 'image/tiff',

    // Videos
    mp4: 'video/mp4',
    webm: 'video/webm',
    mov: 'video/quicktime',
    avi: 'video/x-msvideo',
    mkv: 'video/x-matroska',
    wmv: 'video/x-ms-wmv',
    flv: 'video/x-flv',

    // Audio
    mp3: 'audio/mpeg',
    wav: 'audio/wav',
    ogg: 'audio/ogg',
    m4a: 'audio/mp4',
    flac: 'audio/flac',
    aac: 'audio/aac',

    // Documents
    pdf: 'application/pdf',
    doc: 'application/msword',
    docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    xls: 'application/vnd.ms-excel',
    xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
    ppt: 'application/vnd.ms-powerpoint',
    pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',

    // Archives
    zip: 'application/zip',
    rar: 'application/vnd.rar',
    '7z': 'application/x-7z-compressed',
    tar: 'application/x-tar',
    gz: 'application/gzip',

    // Text
    txt: 'text/plain',
    html: 'text/html',
    htm: 'text/html',
    css: 'text/css',
    js: 'text/javascript',
    json: 'application/json',
    xml: 'application/xml',
    csv: 'text/csv',
    md: 'text/markdown',

    // Other
    wasm: 'application/wasm',
    woff: 'font/woff',
    woff2: 'font/woff2',
    ttf: 'font/ttf',
    otf: 'font/otf',
    eot: 'application/vnd.ms-fontobject',
  };

  return mimeTypes[ext];
}

/**
 * Calculate the number of chunks for a file
 *
 * @param fileSize - Total file size in bytes
 * @param chunkSize - Size of each chunk in bytes
 * @returns Number of chunks
 */
export function calculateChunkCount(fileSize: number, chunkSize: number): number {
  return Math.ceil(fileSize / chunkSize);
}

/**
 * Get the byte range for a specific chunk
 *
 * @param chunkIndex - Zero-based chunk index
 * @param chunkSize - Size of each chunk in bytes
 * @param fileSize - Total file size in bytes
 * @returns Object with start and end byte offsets
 */
export function getChunkRange(
  chunkIndex: number,
  chunkSize: number,
  fileSize: number
): { start: number; end: number } {
  const start = chunkIndex * chunkSize;
  const end = Math.min(start + chunkSize, fileSize);
  return { start, end };
}
