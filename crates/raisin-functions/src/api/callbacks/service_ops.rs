// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Service operation callback type definitions
//!
//! Includes HTTP, Event, AI, PDF, Resource, Task, and Function execution callbacks.

use raisin_error::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

// ========== HTTP Operation Callbacks ==========

/// Callback for HTTP requests
pub type HttpRequestCallback = Arc<
    dyn Fn(
            String, // method
            String, // url
            Value,  // options
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Event Operation Callbacks ==========

/// Callback for event emission
pub type EmitEventCallback = Arc<
    dyn Fn(
            String, // event_type
            Value,  // data
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

// ========== AI Operation Callbacks ==========

/// Callback for AI completion
pub type AICompletionCallback = Arc<
    dyn Fn(
            Value, // request
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for AI embedding generation
///
/// Generates vector embeddings for text or image data.
///
/// # Request Format
/// ```json
/// {
///   "model": "local:clip",         // or "openai:text-embedding-3-small"
///   "input": "base64-or-text",     // Base64 image data or text string
///   "input_type": "image"          // or "text" (optional, auto-detected)
/// }
/// ```
///
/// # Response Format
/// ```json
/// {
///   "embedding": [0.1, 0.2, ...],  // Vector of f32 values
///   "model": "clip",               // Model used
///   "dimensions": 512              // Vector dimension
/// }
/// ```
pub type AIEmbedCallback = Arc<
    dyn Fn(
            Value, // request
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for listing AI models
pub type AIListModelsCallback = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send>>
        + Send
        + Sync,
>;

/// Callback for getting default AI model
pub type AIGetDefaultModelCallback = Arc<
    dyn Fn(
            String, // use_case
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<String>>> + Send>>
        + Send
        + Sync,
>;

// ========== PDF Processing Callbacks ==========

/// Callback for PDF processing from storage key.
///
/// Processes a PDF file stored in binary storage (filesystem or S3) and returns
/// extracted text, page metadata, and optionally a thumbnail.
///
/// # Request Format
/// ```json
/// {
///   "storageKey": "uploads/tenant/doc.pdf",
///   "options": {
///     "ocr": true,               // Enable OCR for scanned pages
///     "ocrLanguages": ["eng"],   // Tesseract language codes
///     "generateThumbnail": true, // Generate first-page thumbnail
///     "thumbnailWidth": 200      // Thumbnail max width
///   }
/// }
/// ```
///
/// # Response Format
/// ```json
/// {
///   "text": "Extracted text...",
///   "pageCount": 5,
///   "isScanned": false,
///   "ocrUsed": false,
///   "extractionMethod": "native",
///   "thumbnail": {
///     "base64": "...",
///     "mimeType": "image/jpeg",
///     "name": "thumbnail.jpg"
///   }
/// }
/// ```
pub type PdfProcessFromStorageCallback = Arc<
    dyn Fn(
            String, // storage_key
            Value,  // options (StoragePdfOptions as JSON)
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Resource Operation Callbacks ==========

/// Callback to get binary data from a storage key.
///
/// Returns base64-encoded binary data for a given storage key.
/// Used by the Resource class in JavaScript to fetch file contents.
///
/// # Arguments
/// - `storage_key`: The storage key (e.g., from `node.properties.file.metadata.storage_key`)
///
/// # Returns
/// - `Result<String>`: Base64-encoded binary data
pub type ResourceGetBinaryCallback = Arc<
    dyn Fn(
            String, // storage_key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
        + Send
        + Sync,
>;

/// Callback to add or update a resource property on a node.
///
/// Uploads binary data (from base64, temp file handle, etc.) and creates
/// a resource property on the node.
///
/// # Arguments
/// - `workspace`: The workspace
/// - `node_path`: Path to the node
/// - `property_path`: Property path (e.g., "thumbnail", "file")
/// - `upload_data`: JSON object with upload data:
///   - `{ "base64": "...", "mimeType": "image/jpeg", "filename": "optional.jpg" }`
///   - `{ "tempHandle": "temp-123" }` for temp files from resize operations
///
/// # Returns
/// - `Result<Value>`: Updated node or resource metadata
pub type NodeAddResourceCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // node_path
            String, // property_path
            Value,  // upload_data
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Task Operation Callbacks ==========

/// Callback for creating human tasks (fire-and-forget)
pub type TaskCreateCallback = Arc<
    dyn Fn(
            Value, // request (task_type, title, assignee, description, options, etc.)
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for updating a task
pub type TaskUpdateCallback = Arc<
    dyn Fn(
            String, // task_id
            Value,  // updates (status, response, etc.)
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for completing a task
pub type TaskCompleteCallback = Arc<
    dyn Fn(
            String, // task_id
            Value,  // response
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for querying tasks
pub type TaskQueryCallback = Arc<
    dyn Fn(
            Value, // query (assignee, status, due_before, etc.)
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>>> + Send>>
        + Send
        + Sync,
>;

// ========== Function Execution Callbacks ==========

/// Context for function execution with tool call handling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionExecuteContext {
    /// Path to the AIToolCall node (for status updates + result creation)
    pub tool_call_path: String,
    /// Workspace where AIToolCall lives
    pub tool_call_workspace: String,
}

/// Callback for raisin.functions.execute(path, args, context)
///
/// This callback:
/// 1. Updates AIToolCall status to 'running'
/// 2. Creates FunctionExecution job
/// 3. Waits for completion
/// 4. Creates AIToolResult node
/// 5. Updates AIToolCall status to 'completed' or 'failed'
/// 6. Returns function result (or error)
pub type FunctionExecuteCallback = Arc<
    dyn Fn(
            String,                 // function_path (e.g., "/functions/tools/get-weather")
            Value,                  // arguments
            FunctionExecuteContext, // tool_call_path, workspace
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for raisin.functions.call(path, args)
///
/// Simple function-to-function call without AI tool call context.
/// This callback:
/// 1. Creates FunctionExecution job
/// 2. Waits for completion
/// 3. Returns function result (or error)
///
/// Unlike `FunctionExecuteCallback`, this does NOT:
/// - Update any AIToolCall status
/// - Create AIToolResult nodes
pub type FunctionCallCallback = Arc<
    dyn Fn(
            String, // function_path (e.g., "/lib/stewardship/is-steward-of")
            Value,  // arguments
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;
