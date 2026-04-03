// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! TypeScript declaration file (.d.ts) generator
//!
//! Generates TypeScript type definitions for the RaisinDB function runtime API.
//! This creates a `raisin.d.ts` file that can be published as `@raisindb/functions-types`
//! for IDE autocomplete and agent assistance.
//!
//! Generated from the shared bindings registry + the Resource/Node wrapper types
//! defined in `api_wrapper.js`.

use crate::runtime::bindings::methods::registry;
use crate::runtime::bindings::registry::{ArgType, ReturnType};

/// Generate the complete TypeScript declaration file
pub fn generate_dts() -> String {
    let reg = registry();
    let mut code = String::with_capacity(16384);

    // Header
    code.push_str(
        r#"/**
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

"#,
    );

    // Static preamble: Resource class, Node interface, and other types
    // that come from api_wrapper.js (not the registry)
    code.push_str(&generate_static_types());

    // Generate namespace declarations from the registry
    code.push_str("// ==========================================================================\n");
    code.push_str("// The raisin Global Object (auto-generated from bindings registry)\n");
    code.push_str("// ==========================================================================\n\n");
    code.push_str("declare namespace raisin {\n");

    // Group methods by category
    let categories = reg.categories();

    for category in &categories {
        if *category == "internal" || *category == "context" {
            continue;
        }

        let methods = reg.methods_by_category(category);
        if methods.is_empty() {
            continue;
        }

        // Handle admin categories as nested namespace
        if category.starts_with("admin_") {
            continue;
        }

        // Handle notify as a direct method
        if *category == "notify" {
            continue;
        }

        // Handle http specially (has convenience wrappers not in registry)
        if *category == "http" {
            code.push_str(&generate_http_namespace());
            continue;
        }

        // Handle nodes specially (has getResource/addResource/beginTransaction from wrapper)
        if *category == "nodes" {
            code.push_str(&generate_nodes_namespace(&methods));
            continue;
        }

        // Handle tx specially (transaction builder pattern)
        if *category == "tx" {
            code.push_str(&generate_tx_namespace(&methods));
            continue;
        }

        // Standard namespace
        code.push_str(&format!("  namespace {} {{\n", category));
        for method in &methods {
            code.push_str(&generate_ts_method(method, 4));
        }
        code.push_str("  }\n\n");
    }

    // Notify as direct method
    let notify_methods = reg.methods_by_category("notify");
    if !notify_methods.is_empty() {
        code.push_str("  /** Send a notification to a user. */\n");
        code.push_str(
            "  function notify(options: NotifyOptions): Promise<any>;\n\n",
        );
    }

    // Admin namespace
    if !reg.methods_by_category("admin_nodes").is_empty()
        || !reg.methods_by_category("admin_sql").is_empty()
    {
        code.push_str(
            "  /** Admin methods that bypass row-level security. Requires requiresAdmin: true in function metadata. */\n",
        );
        code.push_str("  namespace admin {\n");

        let admin_nodes = reg.methods_by_category("admin_nodes");
        if !admin_nodes.is_empty() {
            code.push_str("    namespace nodes {\n");
            for method in &admin_nodes {
                code.push_str(&generate_ts_method(method, 6));
            }
            code.push_str("    }\n");
        }

        let admin_sql = reg.methods_by_category("admin_sql");
        if !admin_sql.is_empty() {
            code.push_str("    namespace sql {\n");
            for method in &admin_sql {
                code.push_str(&generate_ts_method(method, 6));
            }
            code.push_str("    }\n");
        }

        code.push_str("  }\n\n");
    }

    // Context getter
    code.push_str("  /** Execution context with tenant, repo, branch, workspace info. */\n");
    code.push_str("  const context: ExecutionContext;\n\n");

    // asAdmin helper
    code.push_str("  /**\n");
    code.push_str("   * Escalate to admin context (bypasses RLS).\n");
    code.push_str("   * Requires `requiresAdmin: true` in function .node.yaml metadata.\n");
    code.push_str("   */\n");
    code.push_str("  function asAdmin(): typeof raisin.admin;\n");

    code.push_str("}\n\n");

    // Transaction interface
    code.push_str("// ==========================================================================\n");
    code.push_str("// Transaction (returned by raisin.nodes.beginTransaction())\n");
    code.push_str("// ==========================================================================\n\n");
    code.push_str(TRANSACTION_INTERFACE);
    code.push('\n');

    // Console
    code.push_str("// ==========================================================================\n");
    code.push_str("// Console (logging)\n");
    code.push_str("// ==========================================================================\n\n");
    code.push_str("declare namespace console {\n");
    code.push_str("  function log(...args: any[]): void;\n");
    code.push_str("  function debug(...args: any[]): void;\n");
    code.push_str("  function warn(...args: any[]): void;\n");
    code.push_str("  function error(...args: any[]): void;\n");
    code.push_str("}\n\n");

    // W3C Fetch API (built-in polyfill)
    code.push_str("// ==========================================================================\n");
    code.push_str("// W3C Fetch API (built-in — no import needed)\n");
    code.push_str("// ==========================================================================\n\n");
    code.push_str("declare function fetch(input: string | Request, init?: RequestInit): Promise<Response>;\n");
    code.push_str("declare function setTimeout(callback: () => void, ms?: number): number;\n");
    code.push_str("declare function clearTimeout(id: number): void;\n");
    code.push_str("declare function setInterval(callback: () => void, ms?: number): number;\n");
    code.push_str("declare function clearInterval(id: number): void;\n\n");

    // Module exports
    code.push_str("/** Standard function export pattern: module.exports = { handler }; */\n");
    code.push_str("declare var module: { exports: Record<string, any> };\n");

    code
}

/// Generate the static type definitions (Resource, Node, etc.)
/// These come from api_wrapper.js, not from the registry.
fn generate_static_types() -> String {
    r#"// ==========================================================================
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

"#
    .to_string()
}

/// Generate the nodes namespace (has extra wrapper methods)
fn generate_nodes_namespace(
    methods: &[&crate::runtime::bindings::registry::ApiMethodDescriptor],
) -> String {
    let mut code = String::new();
    code.push_str("  namespace nodes {\n");

    for method in methods {
        // Override return types for methods that return wrapped nodes
        let return_override = match method.internal_name {
            "nodes_get" | "nodes_getById" => Some("Promise<RaisinNode | null>"),
            "nodes_create" | "nodes_update" | "nodes_move" => Some("Promise<RaisinNode>"),
            "nodes_query" | "nodes_getChildren" => Some("Promise<RaisinNode[]>"),
            _ => None,
        };

        if let Some(ret) = return_override {
            code.push_str(&generate_ts_method_with_return(method, 4, ret));
        } else {
            code.push_str(&generate_ts_method(method, 4));
        }
    }

    // beginTransaction - from api_wrapper.js, not in registry
    code.push_str("    /**\n");
    code.push_str("     * Start a transaction for atomic multi-node operations.\n");
    code.push_str("     * @example\n");
    code.push_str("     * const tx = raisin.nodes.beginTransaction();\n");
    code.push_str("     * tx.create(workspace, parentPath, data);\n");
    code.push_str("     * tx.commit();\n");
    code.push_str("     */\n");
    code.push_str("    function beginTransaction(): Transaction;\n");

    code.push_str("  }\n\n");
    code
}

/// Generate the transaction namespace as an interface
fn generate_tx_namespace(
    methods: &[&crate::runtime::bindings::registry::ApiMethodDescriptor],
) -> String {
    let mut code = String::new();
    // Don't emit tx as a namespace - it's exposed via beginTransaction() -> Transaction
    // But we need the Transaction interface
    // Skip individual tx methods - they're already on the Transaction interface
    let _ = methods; // suppress unused warning

    code.push_str("  // Transaction methods are accessed via raisin.nodes.beginTransaction()\n");
    code
}

/// Generate the HTTP namespace (has convenience wrappers)
fn generate_http_namespace() -> String {
    r#"  namespace http {
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

"#
    .to_string()
}

/// Generate a single TypeScript method declaration
fn generate_ts_method(
    method: &crate::runtime::bindings::registry::ApiMethodDescriptor,
    indent: usize,
) -> String {
    let return_type = map_return_type(&method.return_type);
    generate_ts_method_with_return(method, indent, &return_type)
}

/// Generate a single TypeScript method declaration with a custom return type
fn generate_ts_method_with_return(
    method: &crate::runtime::bindings::registry::ApiMethodDescriptor,
    indent: usize,
    return_type: &str,
) -> String {
    let indent_str = " ".repeat(indent);
    let mut code = String::new();

    // Build typed argument list
    let args: Vec<String> = method
        .args
        .iter()
        .map(|a| format!("{}{}: {}", a.name, optional_marker(a.arg_type), map_arg_type(a.arg_type)))
        .collect();
    let args_str = args.join(", ");

    code.push_str(&format!(
        "{}function {}({}): {};\n",
        indent_str, method.js_name, args_str, return_type
    ));

    code
}

/// Map ArgType to TypeScript type string
fn map_arg_type(arg_type: ArgType) -> &'static str {
    match arg_type {
        ArgType::String => "string",
        ArgType::OptionalString => "string | null",
        ArgType::Json => "any",
        ArgType::OptionalJson => "any | null",
        ArgType::U32 | ArgType::I64 => "number",
        ArgType::OptionalU32 | ArgType::OptionalI64 => "number | null",
        ArgType::Bool => "boolean",
        ArgType::OptionalBool => "boolean | null",
        ArgType::StringArray => "string[]",
        ArgType::JsonArray => "any[]",
    }
}

/// Return "?" for optional args
fn optional_marker(arg_type: ArgType) -> &'static str {
    match arg_type {
        ArgType::OptionalString
        | ArgType::OptionalJson
        | ArgType::OptionalU32
        | ArgType::OptionalI64
        | ArgType::OptionalBool => "?",
        _ => "",
    }
}

/// Map ReturnType to TypeScript return type string (wrapped in Promise)
fn map_return_type(return_type: &ReturnType) -> String {
    match return_type {
        ReturnType::Json => "Promise<any>".to_string(),
        ReturnType::OptionalJson => "Promise<any | null>".to_string(),
        ReturnType::JsonArray => "Promise<any[]>".to_string(),
        ReturnType::Bool => "Promise<boolean>".to_string(),
        ReturnType::I64 => "Promise<number>".to_string(),
        ReturnType::String => "Promise<string>".to_string(),
        ReturnType::Void => "Promise<void>".to_string(),
    }
}

// ==========================================================================
// Transaction interface (static — matches api_wrapper.js)
// ==========================================================================

const TRANSACTION_INTERFACE: &str = r#"interface Transaction {
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
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn test_generate_dts() {
        let dts = generate_dts();

        // Basic structure checks
        assert!(dts.contains("declare class Resource"), "Should have Resource class");
        assert!(dts.contains("interface RaisinNode"), "Should have RaisinNode interface");
        assert!(dts.contains("declare namespace raisin"), "Should have raisin namespace");
        assert!(dts.contains("namespace nodes"), "Should have nodes namespace");
        assert!(dts.contains("namespace sql"), "Should have sql namespace");
        assert!(dts.contains("namespace ai"), "Should have ai namespace");
        assert!(dts.contains("namespace http"), "Should have http namespace");
        assert!(dts.contains("namespace admin"), "Should have admin namespace");
        assert!(dts.contains("function beginTransaction"), "Should have beginTransaction");
        assert!(dts.contains("resize(options: ResizeOptions)"), "Should have resize method");
        assert!(dts.contains("processDocument"), "Should have processDocument method");
        assert!(dts.contains("getResource"), "Should have getResource method");
        assert!(dts.contains("addResource"), "Should have addResource method");
        assert!(dts.contains("declare var module"), "Should have module.exports");

        // Should NOT contain internal runtime details
        assert!(!dts.contains("__raisin_internal"), "Should not expose internal APIs");

        println!("Generated .d.ts: {} lines", dts.lines().count());
    }

    #[test]
    fn test_generate_and_write_dts() {
        let dts = generate_dts();

        // Write to the functions-types package
        let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/raisindb-functions-types/raisin.d.ts");

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&out_path, &dts).expect("Failed to write raisin.d.ts");

        println!("Wrote {} bytes to {}", dts.len(), out_path.display());
    }
}
