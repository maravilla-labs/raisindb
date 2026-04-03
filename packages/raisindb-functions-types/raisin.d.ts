/**
 * RaisinDB Server-Side Function Runtime Type Definitions
 *
 * These types describe the `raisin` global object available inside
 * RaisinDB server-side functions (QuickJS runtime).
 *
 * This is NOT Node.js — no `Buffer`, `fs`, `require()`, or npm modules.
 *
 * Available globals (beyond the raisin.* API):
 * - ES module imports with relative paths: import { foo } from './utils.js'
 * - W3C Fetch API: fetch(), Request, Response, Headers, ReadableStream, AbortController, FormData
 * - Timers: setTimeout, clearTimeout, setInterval, clearInterval
 * - Console: console.log, console.debug, console.warn, console.error
 *
 * Auto-generated from: crates/raisin-functions/src/runtime/bindings/
 */

// ==========================================================================
// Resource & Node Types (from api_wrapper.js — not in the bindings registry)
// ==========================================================================

/**
 * A binary file resource attached to a node property.
 * Returned by `node.getResource('./file')`.
 *
 * Provides built-in server-side image resizing and PDF processing.
 * This is NOT Node.js — no Buffer, fs, require(), or npm modules.
 * fetch() IS available (W3C Fetch API). ES module imports with relative paths are supported.
 * Use resource.resize() for images and resource.processDocument() for PDFs.
 */
declare class Resource {
  /** Unique identifier */
  readonly uuid: string;
  /** Original filename */
  readonly name: string;
  /** File size in bytes */
  readonly size: number;
  /** MIME type (e.g., "image/jpeg", "application/pdf") */
  readonly mimeType: string;
  /** Storage metadata */
  readonly metadata: Record<string, any>;
  /** Internal storage key */
  readonly storageKey: string | null;

  /** Get binary data as base64 string. */
  getBinary(): Promise<string>;

  /** Get as data URL (data:mime;base64,...). */
  toDataUrl(): Promise<string>;

  /**
   * Resize image server-side. Returns a NEW Resource with the resized data.
   * This is the ONLY way to create thumbnails. Do NOT use sharp, jimp, Canvas,
   * or any external library — they do not exist in this runtime.
   *
   * @example
   * const resource = node.getResource('./file');
   * const thumbnail = await resource.resize({ maxWidth: 200, format: 'jpeg', quality: 80 });
   * await node.addResource('./thumbnail', thumbnail);
   */
  resize(options: ResizeOptions): Promise<Resource>;

  /**
   * Convert a PDF page to an image. Returns a new Resource.
   * Only works with PDF files (mimeType contains "pdf").
   */
  toImage(options?: PdfToImageOptions): Promise<Resource>;

  /** Get page count for PDF files. Only works with PDFs. */
  getPageCount(): Promise<number>;

  /**
   * Process PDF document server-side: extract text, OCR, generate thumbnail.
   * Uses storage-key-based API (no base64 overhead). Only works with PDFs.
   *
   * @example
   * const resource = node.getResource('./file');
   * const result = await resource.processDocument({ generateThumbnail: true, thumbnailWidth: 200 });
   * if (result.thumbnail) {
   *   await node.addResource('./thumbnail', result.thumbnail);
   * }
   */
  processDocument(options?: ProcessDocumentOptions): Promise<DocumentResult>;
}

interface ResizeOptions {
  /** Maximum width in pixels */
  maxWidth?: number;
  /** Maximum height in pixels */
  maxHeight?: number;
  /** Output format */
  format?: 'jpeg' | 'png' | 'webp';
  /** Quality 1-100 (JPEG/WebP only) */
  quality?: number;
}

interface PdfToImageOptions {
  /** Page number (0-indexed, default 0) */
  page?: number;
  /** Maximum width in pixels */
  maxWidth?: number;
  /** Output format (default 'jpeg') */
  format?: 'jpeg' | 'png' | 'webp';
  /** Quality 1-100 */
  quality?: number;
}

interface ProcessDocumentOptions {
  /** Enable OCR for scanned PDFs */
  ocr?: boolean;
  /** OCR languages (default ["eng"]) */
  ocrLanguages?: string[];
  /** Generate a thumbnail of the first page */
  generateThumbnail?: boolean;
  /** Thumbnail width in pixels */
  thumbnailWidth?: number;
}

interface DocumentResult {
  /** Extracted text content */
  text: string;
  /** Number of pages */
  pageCount: number;
  /** Whether the PDF appears to be scanned */
  isScanned: boolean;
  /** Whether OCR was used */
  ocrUsed: boolean;
  /** Extraction method used */
  extractionMethod: string;
  /** Thumbnail Resource (if generateThumbnail was true) */
  thumbnail?: Resource;
}

/**
 * A node returned by raisin.nodes.get() and similar methods.
 * Includes helper methods for binary resource operations.
 */
interface RaisinNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  archetype?: string;
  properties: Record<string, any>;
  created_at?: string;
  updated_at?: string;

  /**
   * Get a Resource handle for a binary property.
   * @param propertyPath - e.g., "./file" or "file"
   * @returns Resource with resize(), processDocument(), etc., or null if not present
   */
  getResource(propertyPath: string): Resource | null;

  /**
   * Upload/store a Resource on a node property.
   * @param propertyPath - Target property, e.g., "./thumbnail"
   * @param data - Resource (from resize()), or { base64, mimeType, name }
   */
  addResource(propertyPath: string, data: Resource | ResourceUploadData | string): Promise<any>;
}

interface ResourceUploadData {
  base64: string;
  mimeType: string;
  name?: string;
}

interface NodeCreateData {
  name?: string;
  path?: string;
  node_type: string;
  properties?: Record<string, any>;
}

/** Execution context available as raisin.context */
interface ExecutionContext {
  tenant_id: string;
  repo_id: string;
  branch: string;
  workspace_id: string;
  actor?: string;
  execution_id?: string;
}

/** Context passed to every function handler */
interface FunctionContext {
  flow_input: {
    event: {
      node_id: string;
      node_type: string;
      node_path: string;
      event_type: string;
    };
    workspace: string;
  };
}

interface NotifyOptions {
  title: string;
  body?: string;
  recipient?: string;
  recipientId?: string;
  priority?: 'low' | 'normal' | 'high';
  type?: string;
  link?: string;
  data?: Record<string, any>;
}

interface HttpOptions {
  method?: string;
  headers?: Record<string, string>;
  body?: any;
  params?: Record<string, string>;
  timeout?: number;
}

interface HttpResponse {
  status: number;
  headers: Record<string, string>;
  body: any;
}

interface AiCompletionRequest {
  model: string;
  messages: Array<{ role: 'system' | 'user' | 'assistant'; content: string }>;
  response_format?: { type: 'json_object'; schema?: any };
  temperature?: number;
  max_tokens?: number;
}

interface AiEmbedRequest {
  model: string;
  input: string | string[];
  input_type?: 'search_document' | 'search_query';
}

// ==========================================================================
// The raisin Global Object (auto-generated from bindings registry)
// ==========================================================================

declare namespace raisin {
  namespace ai {
    function completion(request: any): Promise<any>;
    function listModels(): Promise<any[]>;
    function getDefaultModel(useCase: string): Promise<any | null>;
    function embed(request: any): Promise<any>;
  }

  namespace crypto {
    function uuid(): Promise<string>;
  }

  namespace date {
    function now(): Promise<string>;
    function timestamp(): Promise<number>;
    function timestampMillis(): Promise<number>;
    function parse(dateStr: string, format?: string | null): Promise<number>;
    function format(timestamp: number, format?: string | null): Promise<string>;
    function addDays(timestamp: number, days: number): Promise<number>;
    function diffDays(ts1: number, ts2: number): Promise<number>;
  }

  namespace events {
    function emit(eventType: string, data: any): Promise<void>;
  }

  namespace functions {
    function execute(functionPath: string, arguments: any, context: any): Promise<any>;
    function call(functionPath: string, arguments: any): Promise<any>;
  }

  namespace http {
    /** Make an HTTP request. */
    function request(method: string, url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP GET */
    function get(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP POST */
    function post(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP PUT */
    function put(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP PATCH */
    function patch(url: string, options?: HttpOptions): Promise<HttpResponse>;
    /** HTTP DELETE */
    function delete(url: string, options?: HttpOptions): Promise<HttpResponse>;
  }

  namespace nodes {
    function get(workspace: string, path: string): Promise<RaisinNode | null>;
    function getById(workspace: string, id: string): Promise<RaisinNode | null>;
    function create(workspace: string, parentPath: string, data: any): Promise<RaisinNode>;
    function update(workspace: string, path: string, data: any): Promise<RaisinNode>;
    function delete(workspace: string, path: string): Promise<void>;
    function updateProperty(workspace: string, nodePath: string, propertyPath: string, value: any): Promise<void>;
    function move(workspace: string, nodePath: string, newParentPath: string): Promise<RaisinNode>;
    function query(workspace: string, query: any): Promise<RaisinNode[]>;
    function getChildren(workspace: string, parentPath: string, limit?: number | null): Promise<RaisinNode[]>;
    function addResource(workspace: string, nodePath: string, propertyPath: string, uploadData: any): Promise<any>;
    /**
     * Start a transaction for atomic multi-node operations.
     * @example
     * const tx = raisin.nodes.beginTransaction();
     * tx.create(workspace, parentPath, data);
     * tx.commit();
     */
    function beginTransaction(): Transaction;
  }

  namespace pdf {
    function processFromStorage(storageKey: string, options: any): Promise<any>;
  }

  namespace resources {
    function getBinary(storageKey: string): Promise<string>;
  }

  namespace sql {
    function query(sql: string, params: any[]): Promise<any>;
    function execute(sql: string, params: any[]): Promise<number>;
  }

  namespace tasks {
    function create(request: any): Promise<any>;
    function update(task_id: string, updates: any): Promise<any>;
    function complete(task_id: string, response: any): Promise<any>;
    function query(query: any): Promise<any[]>;
  }

  // Transaction methods are accessed via raisin.nodes.beginTransaction()
  /** Send a notification to a user. */
  function notify(options: NotifyOptions): Promise<any>;

  /** Admin methods that bypass row-level security. Requires requiresAdmin: true in function metadata. */
  namespace admin {
    namespace nodes {
      function get(workspace: string, path: string): Promise<any | null>;
      function getById(workspace: string, id: string): Promise<any | null>;
      function create(workspace: string, parentPath: string, data: any): Promise<any>;
      function update(workspace: string, path: string, data: any): Promise<any>;
      function delete(workspace: string, path: string): Promise<void>;
      function updateProperty(workspace: string, nodePath: string, propertyPath: string, value: any): Promise<void>;
      function query(workspace: string, query: any): Promise<any[]>;
      function getChildren(workspace: string, parentPath: string, limit?: number | null): Promise<any[]>;
    }
    namespace sql {
      function query(sql: string, params: any[]): Promise<any>;
      function execute(sql: string, params: any[]): Promise<number>;
    }
  }

  /** Execution context with tenant, repo, branch, workspace info. */
  const context: ExecutionContext;

  /**
   * Escalate to admin context (bypasses RLS).
   * Requires `requiresAdmin: true` in function .node.yaml metadata.
   */
  function asAdmin(): typeof raisin.admin;
}

// ==========================================================================
// Transaction (returned by raisin.nodes.beginTransaction())
// ==========================================================================

interface Transaction {
  create(workspace: string, parentPath: string, data: NodeCreateData): any;
  add(workspace: string, data: NodeCreateData): any;
  put(workspace: string, data: NodeCreateData): void;
  upsert(workspace: string, data: NodeCreateData): void;
  createDeep(workspace: string, parentPath: string, data: NodeCreateData, parentNodeType?: string): any;
  upsertDeep(workspace: string, data: NodeCreateData, parentNodeType?: string): void;
  update(workspace: string, path: string, data: Partial<NodeCreateData>): void;
  delete(workspace: string, path: string): void;
  deleteById(workspace: string, id: string): void;
  get(workspace: string, id: string): RaisinNode | null;
  getByPath(workspace: string, path: string): RaisinNode | null;
  listChildren(workspace: string, parentPath: string): RaisinNode[];
  updateProperty(workspace: string, nodePath: string, propertyPath: string, value: any): void;
  setActor(actor: string): void;
  setMessage(message: string): void;
  commit(): void;
  rollback(): void;
}

// ==========================================================================
// Console (logging)
// ==========================================================================

declare namespace console {
  function log(...args: any[]): void;
  function debug(...args: any[]): void;
  function warn(...args: any[]): void;
  function error(...args: any[]): void;
}

// ==========================================================================
// W3C Fetch API (built-in — no import needed)
// ==========================================================================

declare function fetch(input: string | Request, init?: RequestInit): Promise<Response>;
declare function setTimeout(callback: () => void, ms?: number): number;
declare function clearTimeout(id: number): void;
declare function setInterval(callback: () => void, ms?: number): number;
declare function clearInterval(id: number): void;

/** Standard function export pattern: module.exports = { handler }; */
declare var module: { exports: Record<string, any> };
