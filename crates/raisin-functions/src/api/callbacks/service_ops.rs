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

// ========== Plugin Binding Callbacks ==========

/// The trusted execution context handed to a [`PluginCallback`] — the identity
/// the calling function runs under, bound SERVER-side (never from guest script).
/// A plugin uses it to scope its outbound call (e.g. the `x-tenant-id` header on
/// a Delivery request), so a tenant can never act as another tenant.
#[derive(Debug, Clone)]
pub struct PluginCallContext {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub workspace_id: Option<String>,
}

/// Callback servicing one method of a [`crate::plugin::FunctionBindingPlugin`].
///
/// Receives the trusted [`PluginCallContext`] and the positional-args JSON array
/// the guest passed to the binding (e.g. `[request]` for
/// `raisin.media.screenshot(request)`), and returns the method's JSON result.
/// Registered in [`RaisinFunctionApiCallbacks::plugin_callbacks`], keyed by the
/// logical method name (`"<namespace>.<method>"`), and dispatched by
/// `RaisinFunctionApi::plugin_call`. The callback runs in trusted core — plugin
/// bindings are NOT subject to the tenant `network_policy`, so a plugin (e.g.
/// a media-processing plugin proxying an internal service) is the ONLY
/// sanctioned way for a tenant function to reach an internal service.
pub type PluginCallback = Arc<
    dyn Fn(
            PluginCallContext,
            Value,
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

/// Callback for listing AI PROVIDERS (not their models).
///
/// Distinct from [`AIListModelsCallback`] because the two answer different
/// questions and only this one can answer "is this slug configured". A provider
/// that accepts arbitrary model ids (Groq, Ollama) may have ZERO registered
/// models, so it is invisible to a model listing while being perfectly usable.
pub type AIListProvidersCallback = Arc<
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
/// Callback to store an EXTRACTION ARTIFACT produced above this process.
///
/// The landing place for text that core cannot produce itself — a `.docx`
/// converted to markdown by a media plugin, called from a trigger function.
/// Writing it as an ordinary property is not an option: the extraction
/// properties are engine-owned and the write-path shield refuses them to tenant
/// code, which is the whole reason this primitive exists.
///
/// Deliberately takes no fingerprint (computed server-side; a second producer
/// of the stamp is a re-extraction loop) and does no chunking (the node write
/// emits `node:updated` and the ordinary indexing path takes it from there).
///
/// # Arguments
/// - `workspace`: workspace holding the asset
/// - `node_ref`: node id, or an absolute path (leading `/`)
/// - `text`: the extracted text
/// - `options`: `{ source?: string, store?: boolean }`
pub type AssetSetExtractionCallback = Arc<
    dyn Fn(
            String, // workspace
            String, // node id or path
            String, // text
            Value,  // options
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

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

/// Callback for raisin.flows.run(flowPath, input)
///
/// Starts a `raisin:Flow` node (functions workspace) by path in the current
/// repository - fire-and-forget. The flow instance is created and a
/// `FlowInstanceExecution` job is queued via the same service the HTTP
/// `POST /api/flows/{repo}/run` handler uses; the callback does NOT wait
/// for the flow to finish.
///
/// Returns `{ "instance_id": "...", "job_id": "...", "status": "queued" }`.
pub type FlowRunCallback = Arc<
    dyn Fn(
            String, // flow_path (e.g., "/flows/fill-shift")
            Value,  // flow input
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Branch Operation Callbacks ==========

/// Callback for `raisin.branches.diff(branch, baseBranch)`.
///
/// Computes the per-node diff of `branch` relative to `baseBranch`'s
/// merge-base — exactly which nodes were added / modified / deleted since
/// the branches diverged (cost is O(commits since fork), not O(total nodes)).
///
/// Returns `{ "common_ancestor": "...", "added": [...], "modified": [...], "deleted": [...] }`.
pub type BranchDiffCallback = Arc<
    dyn Fn(
            String, // branch (e.g., "feature/new-ui")
            String, // base_branch (e.g., "main")
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for `raisin.branches.compare(branch, baseBranch)`.
///
/// Calculates branch divergence (commits ahead/behind, like Git's
/// divergence tracking) of `branch` relative to `baseBranch`.
///
/// Returns `{ "ahead": u64, "behind": u64, "common_ancestor": "..." }`.
pub type BranchCompareCallback = Arc<
    dyn Fn(
            String, // branch
            String, // base_branch
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for `raisin.branches.copyNodes(sourceBranch, targetBranch, opts)`.
///
/// Copies a node set (optionally recursive) from one branch onto another,
/// preserving node ids, in one atomic commit on the target branch. `opts`
/// carries `{ workspace, roots, recursive?, deleteMissing? }`.
///
/// Returns `{ "copied": usize, "deleted": usize, "revision": "...", "changes": [...] }`.
pub type BranchCopyNodesCallback = Arc<
    dyn Fn(
            String, // source_branch
            String, // target_branch
            Value,  // options: { workspace, roots, recursive?, deleteMissing? }
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Scheduled Invocation Callbacks ==========

/// Callback for `raisin.scheduler.schedule(request)`.
///
/// Registers a one-shot scheduled invocation of a function or flow. The
/// request carries `{ targetKind, targetPath, input?, runAt, externalKey?,
/// branch?, workspace?, maxRetries? }`; `runAt` is RFC3339 and a past time
/// dispatches immediately.
///
/// Returns `{ "job_id": "...", "invocation_id": "...", "status": "scheduled", "run_at": "..." }`.
pub type SchedulerScheduleCallback = Arc<
    dyn Fn(
            Value, // request
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for `raisin.scheduler.cancel(jobIdOrKey)`.
///
/// Cancels a pending scheduled invocation, addressed by job id or by the
/// caller-supplied external key.
///
/// Returns `{ "job_id": "...", "status": "cancelled" }`.
pub type SchedulerCancelCallback = Arc<
    dyn Fn(
            String, // job_id or external key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for `raisin.scheduler.list(filter?)`.
///
/// Lists this repository's scheduled invocations; `filter` may carry
/// `{ externalKey?, status? }`.
///
/// Returns `{ "invocations": [...] }`.
pub type SchedulerListCallback = Arc<
    dyn Fn(
            Value, // filter
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

/// Callback for `raisin.scheduler.get(jobIdOrKey)`.
///
/// Fetches a single scheduled invocation by job id or external key.
pub type SchedulerGetCallback = Arc<
    dyn Fn(
            String, // job_id or external key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Platform Hook Callbacks ==========

/// Callback for `raisin.platform.hook(name, payload?)`.
///
/// Fires an OPERATOR-configured endpoint (`[platform.hooks.<name>]` in the
/// server config) with a JSON payload; the tenant and repo ids are stamped in
/// by the runtime. This is how a function reaches platform services on
/// addresses `raisin.http.fetch` must keep refusing.
///
/// Returns `{ "ok": bool, "status": u16, "body": <json|string> }`.
pub type PlatformHookCallback = Arc<
    dyn Fn(
            String, // hook name
            Value,  // payload
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;

// ========== Integration / Mount Operation Callbacks ==========

/// Callback for `raisin.integrations.sync_now(mountId, mode?)`.
///
/// Enqueues a one-shot `VirtualMountSync` job (deduped per mount via the
/// `vmount-sync:{mount_id}` key) and returns
/// `{ "job_id": String|null, "status": "queued"|"already_running" }`.
pub type IntegrationsSyncNowCallback = Arc<
    dyn Fn(
            String,         // mount_id
            Option<String>, // mode ("delta" | "full")
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>
        + Send
        + Sync,
>;
