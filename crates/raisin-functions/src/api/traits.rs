// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! FunctionApi trait definition
//!
//! This module defines the API surface exposed to user-defined functions.
//! The API mirrors the structure of raisin-client-js for familiarity.

use async_trait::async_trait;
use raisin_error::Result;
use serde_json::Value;

use super::callbacks::FunctionExecuteContext;

/// API surface exposed to functions
///
/// This trait defines all operations available to user-defined functions.
/// Implementations provide access to RaisinDB features in a sandboxed manner.
///
/// # JavaScript API Shape
///
/// In JavaScript functions, this is exposed as the `raisin` global object:
///
/// ```javascript
/// // Node operations
/// const node = await raisin.nodes.get("default", "/my-path");
/// await raisin.nodes.create("default", "/parent", { type: "Page", properties: {} });
/// await raisin.nodes.update("default", "/my-path", { properties: { title: "New" } });
/// await raisin.nodes.delete("default", "/my-path");
/// const nodes = await raisin.nodes.query("default", { node_type: "Page", limit: 10 });
///
/// // SQL operations
/// const result = await raisin.sql.query("SELECT * FROM nodes WHERE node_type = $1", ["Page"]);
/// const count = await raisin.sql.execute("UPDATE nodes SET properties = $1 WHERE id = $2", [props, id]);
///
/// // HTTP operations (allowlisted URLs only)
/// const response = await raisin.http.fetch("https://api.example.com/data", {
///     method: "POST",
///     body: { key: "value" },
///     headers: { "Authorization": "Bearer token" }
/// });
///
/// // AI operations
/// const response = await raisin.ai.completion({
///     model: "gpt-4o",
///     messages: [
///         { role: "system", content: "You are helpful" },
///         { role: "user", content: "Hello!" }
///     ],
///     temperature: 0.7,
///     max_tokens: 1000
/// });
/// const models = await raisin.ai.listModels();
/// const defaultModel = await raisin.ai.getDefaultModel("chat");
///
/// // Events
/// await raisin.events.emit("custom:event", { data: "value" });
///
/// // Logging
/// console.log("Info message");
/// console.error("Error message");
///
/// // Context
/// const tenant = raisin.context.tenant_id;
/// const branch = raisin.context.branch;
/// ```
#[async_trait]
pub trait FunctionApi: Send + Sync {
    // ========== Node Operations ==========

    /// Get a node by path
    async fn node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>>;

    /// Get a node by ID
    async fn node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>>;

    /// Create a new node
    async fn node_create(&self, workspace: &str, parent_path: &str, data: Value) -> Result<Value>;

    /// Update a node
    async fn node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value>;

    /// Delete a node
    async fn node_delete(&self, workspace: &str, path: &str) -> Result<()>;

    /// Update a specific property on a node by path
    async fn node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()>;

    /// Move a node to a new parent path
    async fn node_move(
        &self,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value>;

    /// Query nodes
    async fn node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>>;

    /// Get children of a node
    async fn node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>>;

    // ========== SQL Operations ==========

    /// Execute a SQL query and return results
    async fn sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value>;

    /// Execute a SQL statement (INSERT, UPDATE, DELETE)
    async fn sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64>;

    // ========== HTTP Operations ==========

    /// Make an HTTP request to an allowlisted URL
    async fn http_request(&self, method: &str, url: &str, options: Value) -> Result<Value>;

    // ========== Event Operations ==========

    /// Emit a custom event
    async fn emit_event(&self, event_type: &str, data: Value) -> Result<()>;

    // ========== AI Operations ==========

    /// Call AI completion
    async fn ai_completion(&self, request: Value) -> Result<Value>;

    /// List available AI models
    async fn ai_list_models(&self) -> Result<Vec<Value>>;

    /// Get default model for a use case
    async fn ai_get_default_model(&self, use_case: &str) -> Result<Option<String>>;

    /// Generate embeddings for text or image input
    async fn ai_embed(&self, request: Value) -> Result<Value>;

    // ========== Resource Operations ==========

    /// Get binary data from storage by key
    async fn resource_get_binary(&self, storage_key: &str) -> Result<String>;

    /// Upload and attach a resource to a node property
    async fn node_add_resource(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        upload_data: Value,
    ) -> Result<Value>;

    // ========== PDF Operations ==========

    /// Process a PDF file from storage (storage-key based, no base64 overhead)
    async fn pdf_process_from_storage(&self, storage_key: &str, options: Value) -> Result<Value>;

    // ========== Task Operations ==========

    /// Create a human task in a user's inbox (fire-and-forget)
    async fn task_create(&self, request: Value) -> Result<Value>;

    /// Update a task
    async fn task_update(&self, task_id: &str, updates: Value) -> Result<Value>;

    /// Complete a task with a response
    async fn task_complete(&self, task_id: &str, response: Value) -> Result<Value>;

    /// Query tasks
    async fn task_query(&self, query: Value) -> Result<Vec<Value>>;

    // ========== Function Operations ==========

    /// Execute another function with tool call lifecycle management
    async fn function_execute(
        &self,
        function_path: &str,
        arguments: Value,
        context: FunctionExecuteContext,
    ) -> Result<Value>;

    /// Call another function directly (function-to-function calls)
    async fn function_call(&self, function_path: &str, arguments: Value) -> Result<Value>;

    // ========== Transaction Operations ==========

    /// Begin a new transaction, returns transaction ID
    async fn tx_begin(&self) -> Result<String>;

    /// Commit a transaction
    async fn tx_commit(&self, tx_id: &str) -> Result<()>;

    /// Rollback a transaction
    async fn tx_rollback(&self, tx_id: &str) -> Result<()>;

    /// Set actor for a transaction
    async fn tx_set_actor(&self, tx_id: &str, actor: &str) -> Result<()>;

    /// Set message for a transaction
    async fn tx_set_message(&self, tx_id: &str, message: &str) -> Result<()>;

    /// Create a node within a transaction (auto-generates ID, builds path from parent+name)
    async fn tx_create(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value>;

    /// Add a node within a transaction (uses provided path, auto-generates ID if missing)
    async fn tx_add(&self, tx_id: &str, workspace: &str, data: Value) -> Result<Value>;

    /// Put a node within a transaction (create or update by ID)
    async fn tx_put(&self, tx_id: &str, workspace: &str, data: Value) -> Result<()>;

    /// Upsert a node within a transaction (create or update by PATH)
    async fn tx_upsert(&self, tx_id: &str, workspace: &str, data: Value) -> Result<()>;

    /// Create a node within a transaction with deep parent creation
    async fn tx_create_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<Value>;

    /// Upsert a node within a transaction with deep parent creation
    async fn tx_upsert_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<()>;

    /// Update a node within a transaction
    async fn tx_update(&self, tx_id: &str, workspace: &str, path: &str, data: Value) -> Result<()>;

    /// Delete a node by path within a transaction
    async fn tx_delete(&self, tx_id: &str, workspace: &str, path: &str) -> Result<()>;

    /// Delete a node by ID within a transaction
    async fn tx_delete_by_id(&self, tx_id: &str, workspace: &str, id: &str) -> Result<()>;

    /// Get a node by ID within a transaction
    async fn tx_get(&self, tx_id: &str, workspace: &str, id: &str) -> Result<Option<Value>>;

    /// Get a node by path within a transaction
    async fn tx_get_by_path(
        &self,
        tx_id: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>>;

    /// List children within a transaction
    async fn tx_list_children(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Value>>;

    /// Move a node to a new parent within a transaction
    async fn tx_move(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value>;

    /// Update a specific property on a node within a transaction
    async fn tx_update_property(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()>;

    // ========== Date/Time Operations ==========

    /// Get current UTC datetime as ISO 8601 string
    fn date_now(&self) -> String;

    /// Get current Unix timestamp in seconds
    fn date_timestamp(&self) -> i64;

    /// Get current Unix timestamp in milliseconds
    fn date_timestamp_millis(&self) -> i64;

    /// Parse a date string to Unix timestamp
    fn date_parse(&self, date_str: &str, format: Option<&str>) -> Result<i64>;

    /// Format a Unix timestamp to string
    fn date_format(&self, timestamp: i64, format: Option<&str>) -> Result<String>;

    /// Add days to a timestamp
    fn date_add_days(&self, timestamp: i64, days: i64) -> Result<i64>;

    /// Get difference in days between two timestamps
    fn date_diff_days(&self, ts1: i64, ts2: i64) -> i64;

    // ========== Logging ==========

    /// Log a message
    fn log(&self, level: &str, message: &str);

    // ========== Context ==========

    /// Get the current execution context
    fn get_context(&self) -> Value;

    // ========== Admin Escalation ==========

    /// Check if this function is allowed to escalate to admin context.
    fn allows_admin_escalation(&self) -> bool;

    // ========== Admin Node Operations ==========

    /// Get a node by path (admin context - bypasses RLS)
    async fn admin_node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>>;

    /// Get a node by ID (admin context - bypasses RLS)
    async fn admin_node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>>;

    /// Create a new node (admin context - bypasses permission checks)
    async fn admin_node_create(
        &self,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value>;

    /// Update a node (admin context - bypasses permission checks)
    async fn admin_node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value>;

    /// Delete a node (admin context - bypasses permission checks)
    async fn admin_node_delete(&self, workspace: &str, path: &str) -> Result<()>;

    /// Update a property (admin context - bypasses permission checks)
    async fn admin_node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()>;

    /// Query nodes (admin context - bypasses RLS)
    async fn admin_node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>>;

    /// Get children of a node (admin context - bypasses RLS)
    async fn admin_node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>>;

    // ========== Admin SQL Operations ==========

    /// Execute a SQL query (admin context - bypasses RLS)
    async fn admin_sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value>;

    /// Execute a SQL statement (admin context - bypasses permission checks)
    async fn admin_sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64>;
}
