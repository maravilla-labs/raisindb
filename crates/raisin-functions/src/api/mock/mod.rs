// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Mock FunctionApi implementation for testing

mod mock_helpers;

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDateTime, TimeZone, Utc};
use raisin_error::Result;
use serde_json::Value;

use super::callbacks::FunctionExecuteContext;
use super::traits::FunctionApi;
use mock_helpers::*;

/// Placeholder API implementation for testing
pub struct MockFunctionApi {
    context: Value,
    logs: std::sync::Mutex<Vec<(String, String)>>,
    /// Real in-process lock manager so lock/inventory mocks behave correctly.
    locks: raisin_locks::InProcessLockManager,
    /// When set, node WRITE operations (create/update/delete/updateProperty/move)
    /// fail with this message — used to test that storage errors propagate to
    /// the function runtime as exceptions instead of success-shaped values.
    node_write_error: Option<String>,
    /// When set, reads/queries/sql/events fail with this message too — used to
    /// test the runtimes' swallowed-error conventions (null / [] / sentinels).
    all_errors: Option<String>,
    /// Scripted `http_request` responses, consumed in order. Empty means "echo
    /// the request back as a 200", the historical behaviour every existing test
    /// relies on. Scripting exists so a function under test can be driven
    /// through a provider's NON-200 statuses (a 412 conflict, a 404, a 403)
    /// without a network.
    http_responses: std::sync::Mutex<std::collections::VecDeque<Value>>,
    /// Every `http_request` made, as `{ method, url, options }`, in order. This
    /// is what lets a test assert the exact URL, headers and body a function
    /// produced rather than just its return value.
    http_calls: std::sync::Mutex<Vec<Value>>,
    /// In-memory secret store: `name -> versions`, append-only, newest last.
    /// A tombstone is a `None` value, so `get` can distinguish "deleted" from
    /// "never existed" exactly as the real store does.
    #[allow(clippy::type_complexity)]
    secrets: std::sync::Mutex<std::collections::HashMap<String, Vec<Option<String>>>>,
    /// Every message passed to `email_send`, in order. Nothing is sent — the
    /// point of recording them is that a test can assert on the exact subject
    /// and body a function produced (a magic link in particular) without a
    /// provider account.
    emails_sent: std::sync::Mutex<Vec<Value>>,
}

impl MockFunctionApi {
    /// Create a new mock API
    pub fn new(context: Value) -> Self {
        Self {
            context,
            logs: std::sync::Mutex::new(Vec::new()),
            locks: raisin_locks::InProcessLockManager::new(),
            node_write_error: None,
            all_errors: None,
            http_responses: std::sync::Mutex::new(std::collections::VecDeque::new()),
            http_calls: std::sync::Mutex::new(Vec::new()),
            secrets: std::sync::Mutex::new(std::collections::HashMap::new()),
            emails_sent: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Every `{ to, subject, text, html? }` handed to `email_send` so far.
    pub fn emails_sent(&self) -> Vec<Value> {
        self.emails_sent.lock().unwrap().clone()
    }

    /// Script the responses `raisin.http.fetch` will return, in order. Each
    /// entry is a full `{ status, headers, body }` value. Once the script is
    /// exhausted the mock falls back to echoing the request as a 200.
    pub fn with_http_responses(self, responses: Vec<Value>) -> Self {
        *self.http_responses.lock().unwrap() = responses.into_iter().collect();
        self
    }

    /// The `{ method, url, options }` of every HTTP request made so far.
    pub fn http_calls(&self) -> Vec<Value> {
        self.http_calls.lock().unwrap().clone()
    }

    /// Make all node write operations fail with the given message.
    pub fn with_node_write_error(mut self, message: impl Into<String>) -> Self {
        self.node_write_error = Some(message.into());
        self
    }

    /// Make reads, queries, SQL, and event emission fail too (in addition to
    /// node writes) with the given message.
    pub fn with_all_errors(mut self, message: impl Into<String>) -> Self {
        self.node_write_error = Some(message.into());
        self.all_errors = self.node_write_error.clone();
        self
    }

    fn check_node_write_error(&self) -> Result<()> {
        match &self.node_write_error {
            Some(msg) => Err(raisin_error::Error::PermissionDenied(msg.clone())),
            None => Ok(()),
        }
    }

    fn check_all_errors(&self) -> Result<()> {
        match &self.all_errors {
            Some(msg) => Err(raisin_error::Error::Internal(msg.clone())),
            None => Ok(()),
        }
    }

    /// Get captured logs
    pub fn get_logs(&self) -> Vec<(String, String)> {
        self.logs.lock().unwrap().clone()
    }
}

#[async_trait]
impl FunctionApi for MockFunctionApi {
    // ========== Node Operations ==========

    async fn node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>> {
        self.check_all_errors()?;
        Ok(Some(mock_node(
            workspace,
            path,
            "mock-node-id",
            "raisin:Page",
        )))
    }

    async fn node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>> {
        self.check_all_errors()?;
        Ok(Some(mock_node(workspace, "/mock-path", id, "raisin:Page")))
    }

    async fn node_history(
        &self,
        _workspace: &str,
        _node_id: &str,
        _limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        Ok(vec![])
    }

    async fn node_create(&self, workspace: &str, parent_path: &str, data: Value) -> Result<Value> {
        self.check_node_write_error()?;
        Ok(mock_created_node(workspace, parent_path, &data))
    }

    async fn node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value> {
        self.check_node_write_error()?;
        Ok(mock_updated_node(workspace, path, &data))
    }

    async fn node_delete(&self, _workspace: &str, _path: &str) -> Result<()> {
        self.check_node_write_error()?;
        Ok(())
    }

    async fn node_update_property(
        &self,
        _workspace: &str,
        _node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        self.check_node_write_error()?;
        tracing::info!(property_path = %property_path, value = ?value, "Mock property update");
        Ok(())
    }

    async fn node_move(
        &self,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        self.check_node_write_error()?;
        Ok(mock_moved_node(workspace, node_path, new_parent_path))
    }

    async fn node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>> {
        self.check_all_errors()?;
        let limit = query.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;
        Ok(mock_query_results(workspace, &query, limit))
    }

    async fn node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        self.check_all_errors()?;
        let count = limit.unwrap_or(3) as usize;
        Ok(mock_children(workspace, parent_path, count))
    }

    async fn node_apply_child_order(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _source_branch: &str,
        _target_branch: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn node_reorder_child(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _position: u32,
    ) -> Result<()> {
        self.check_all_errors()?;
        Ok(())
    }

    async fn node_move_child_relative(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _reference_child_name: &str,
        _before: bool,
    ) -> Result<()> {
        self.check_all_errors()?;
        Ok(())
    }

    // ========== SQL Operations ==========

    async fn sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value> {
        self.check_all_errors()?;
        Ok(serde_json::json!({
            "columns": ["id", "name"], "rows": [["1", "test"]], "row_count": 1,
            "_debug": { "sql": sql, "params": params }
        }))
    }

    async fn sql_execute(&self, _sql: &str, _params: Vec<Value>) -> Result<i64> {
        self.check_all_errors()?;
        Ok(1)
    }

    // ========== HTTP / Event Operations ==========

    async fn http_request(&self, method: &str, url: &str, options: Value) -> Result<Value> {
        self.http_calls.lock().unwrap().push(serde_json::json!({
            "method": method, "url": url, "options": options.clone()
        }));
        if let Some(scripted) = self.http_responses.lock().unwrap().pop_front() {
            return Ok(scripted);
        }
        Ok(serde_json::json!({
            "status": 200, "headers": {},
            "body": { "_mock": true, "method": method, "url": url, "options": options }
        }))
    }

    async fn emit_event(&self, event_type: &str, data: Value) -> Result<()> {
        self.check_all_errors()?;
        tracing::info!(event_type = %event_type, data = ?data, "Mock event emitted");
        Ok(())
    }

    // ========== AI Operations ==========

    async fn ai_completion(&self, request: Value) -> Result<Value> {
        let _ = request
            .get("messages")
            .and_then(|m| m.as_array())
            .ok_or_else(|| raisin_error::Error::Validation("Missing messages".to_string()))?;
        Ok(mock_ai_completion(&request))
    }

    async fn ai_list_providers(&self) -> Result<Vec<Value>> {
        Ok(vec![
            serde_json::json!({
                "slug": "openai", "kind": "openai", "enabled": true, "model_count": 1
            }),
            serde_json::json!({
                "slug": "anthropic", "kind": "anthropic", "enabled": true, "model_count": 1
            }),
        ])
    }

    async fn ai_list_models(&self) -> Result<Vec<Value>> {
        Ok(vec![
            serde_json::json!({
                "id": "gpt-4o", "name": "GPT-4 Optimized", "provider": "openai",
                "use_cases": ["chat", "completion"],
                "capabilities": { "chat": true, "streaming": true, "tools": true, "vision": true }
            }),
            serde_json::json!({
                "id": "claude-3-5-sonnet", "name": "Claude 3.5 Sonnet", "provider": "anthropic",
                "use_cases": ["chat", "agent"],
                "capabilities": { "chat": true, "streaming": true, "tools": true, "vision": true }
            }),
        ])
    }

    async fn ai_get_default_model(&self, use_case: &str) -> Result<Option<String>> {
        match use_case {
            "chat" | "completion" => Ok(Some("gpt-4o".to_string())),
            "agent" => Ok(Some("claude-3-5-sonnet".to_string())),
            "embedding" => Ok(Some("text-embedding-3-small".to_string())),
            _ => Ok(None),
        }
    }

    async fn ai_embed(&self, request: Value) -> Result<Value> {
        let model = request
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("text-embedding-3-small");
        let embedding: Vec<f32> = (0..512).map(|i| (i as f32 * 0.001).sin()).collect();
        Ok(serde_json::json!({ "embedding": embedding, "model": model, "dimensions": 512 }))
    }

    // ========== Resource / PDF Operations ==========

    async fn resource_get_binary(&self, storage_key: &str) -> Result<String> {
        tracing::info!(storage_key = %storage_key, "Mock resource_get_binary");
        Ok("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".to_string())
    }

    async fn node_add_resource(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        upload_data: Value,
    ) -> Result<Value> {
        tracing::info!(workspace = %workspace, node_path = %node_path, property_path = %property_path, "Mock node_add_resource");
        Ok(mock_resource_result(
            workspace,
            node_path,
            property_path,
            &upload_data,
        ))
    }

    async fn asset_set_extraction(
        &self,
        workspace: &str,
        node_ref: &str,
        text: &str,
        options: Value,
    ) -> Result<Value> {
        tracing::info!(
            workspace = %workspace, node_ref = %node_ref, chars = text.chars().count(),
            options = ?options, "Mock asset_set_extraction"
        );
        Ok(serde_json::json!({
            "status": if text.trim().is_empty() { "empty" } else { "ok" },
            "source": options.get("source").and_then(|v| v.as_str()).unwrap_or("plugin"),
            "chars": text.chars().count(),
            "stored": !text.trim().is_empty(),
        }))
    }

    async fn asset_reextract(&self, workspace: &str, node_ref: &str) -> Result<Value> {
        tracing::info!(workspace = %workspace, node_ref = %node_ref, "Mock asset_reextract");
        Ok(serde_json::json!({ "queued": true, "node": node_ref }))
    }

    async fn asset_ensure_content(&self, workspace: &str, node_ref: &str) -> Result<Value> {
        tracing::info!(workspace = %workspace, node_ref = %node_ref, "Mock asset_ensure_content");
        Ok(serde_json::json!({ "status": "already_present" }))
    }

    async fn asset_signed_url(
        &self,
        workspace: &str,
        node_ref: &str,
        options: Value,
    ) -> Result<Value> {
        tracing::info!(
            workspace = %workspace, node_ref = %node_ref, options = ?options,
            "Mock asset_signed_url"
        );
        let command = options
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("display");
        let property = options
            .get("property")
            .and_then(|v| v.as_str())
            .unwrap_or("file");
        Ok(serde_json::json!({
            "url": format!(
                "https://mock.invalid/api/repository/repo/main/head/{workspace}{node_ref}\
        /raisin:{command}?sig=mock&exp=0"
            ),
            "expiresAt": null,
            "expiresIn": 300,
            "command": command,
            "property": property,
            "path": node_ref,
        }))
    }

    async fn pdf_process_from_storage(&self, storage_key: &str, options: Value) -> Result<Value> {
        tracing::info!(storage_key = %storage_key, options = ?options, "Mock pdf_process_from_storage");
        Ok(serde_json::json!({
            "text": "Mock extracted text from PDF document.", "pageCount": 3,
            "isScanned": false, "ocrUsed": false, "extractionMethod": "native",
            "thumbnail": { "base64": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==", "mimeType": "image/jpeg", "name": "thumbnail.jpg" }
        }))
    }

    // ========== Task Operations ==========

    async fn task_create(&self, request: Value) -> Result<Value> {
        let task_id = uuid::Uuid::new_v4().to_string();
        let assignee = request
            .get("assignee")
            .and_then(|a| a.as_str())
            .unwrap_or("/users/unknown");
        let task_path = format!(
            "{}/inbox/task-{}-{}",
            assignee,
            &task_id[..8],
            chrono::Utc::now().timestamp()
        );
        tracing::info!(task_id = %task_id, task_path = %task_path, "Mock task create");
        Ok(serde_json::json!({ "task_id": task_id, "task_path": task_path }))
    }

    async fn task_update(&self, task_id: &str, updates: Value) -> Result<Value> {
        tracing::info!(task_id = %task_id, updates = ?updates, "Mock task update");
        Ok(
            serde_json::json!({ "id": task_id, "status": updates.get("status").and_then(|s| s.as_str()).unwrap_or("pending"), "updated": true }),
        )
    }

    async fn task_complete(&self, task_id: &str, response: Value) -> Result<Value> {
        tracing::info!(task_id = %task_id, response = ?response, "Mock task complete");
        Ok(
            serde_json::json!({ "id": task_id, "status": "completed", "response": response, "responded_at": chrono::Utc::now().to_rfc3339() }),
        )
    }

    async fn task_query(&self, query: Value) -> Result<Vec<Value>> {
        tracing::info!(query = ?query, "Mock task query");
        Ok(vec![
            serde_json::json!({ "id": "mock-task-1", "task_type": "approval", "title": "Mock Task 1", "status": "pending" }),
            serde_json::json!({ "id": "mock-task-2", "task_type": "action", "title": "Mock Task 2", "status": "pending" }),
        ])
    }

    // ========== Function Execution ==========

    async fn function_execute(
        &self,
        function_path: &str,
        arguments: Value,
        _context: FunctionExecuteContext,
    ) -> Result<Value> {
        tracing::info!(function_path = %function_path, arguments = ?arguments, "Mock function execute");
        Ok(
            serde_json::json!({ "success": true, "function_path": function_path, "arguments": arguments, "result": "mock execution result" }),
        )
    }

    async fn function_call(&self, function_path: &str, arguments: Value) -> Result<Value> {
        tracing::info!(function_path = %function_path, arguments = ?arguments, "Mock function call");
        Ok(
            serde_json::json!({ "success": true, "function_path": function_path, "arguments": arguments, "result": "mock call result" }),
        )
    }

    // ========== Transaction Operations ==========

    async fn tx_begin(&self) -> Result<String> {
        Ok(format!("mock-tx-{}", uuid::Uuid::new_v4()))
    }
    async fn tx_commit(&self, _tx_id: &str) -> Result<()> {
        Ok(())
    }
    async fn tx_rollback(&self, _tx_id: &str) -> Result<()> {
        Ok(())
    }
    async fn tx_set_actor(&self, _tx_id: &str, _actor: &str) -> Result<()> {
        Ok(())
    }
    async fn tx_set_message(&self, _tx_id: &str, _message: &str) -> Result<()> {
        Ok(())
    }

    async fn tx_create(
        &self,
        _tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value> {
        Ok(mock_tx_created_node(workspace, parent_path, &data))
    }

    async fn tx_add(&self, _tx_id: &str, workspace: &str, data: Value) -> Result<Value> {
        Ok(mock_tx_added_node(workspace, &data))
    }

    async fn tx_put(&self, _tx_id: &str, _workspace: &str, _data: Value) -> Result<()> {
        Ok(())
    }
    async fn tx_upsert(&self, _tx_id: &str, _workspace: &str, _data: Value) -> Result<()> {
        Ok(())
    }

    async fn tx_create_deep(
        &self,
        _tx_id: &str,
        _workspace: &str,
        _parent_path: &str,
        data: Value,
        _parent_node_type: &str,
    ) -> Result<Value> {
        Ok(data)
    }

    async fn tx_upsert_deep(
        &self,
        _tx_id: &str,
        _workspace: &str,
        _data: Value,
        _parent_node_type: &str,
    ) -> Result<()> {
        Ok(())
    }
    async fn tx_update(
        &self,
        _tx_id: &str,
        _workspace: &str,
        _path: &str,
        _data: Value,
    ) -> Result<()> {
        Ok(())
    }
    async fn tx_delete(&self, _tx_id: &str, _workspace: &str, _path: &str) -> Result<()> {
        Ok(())
    }
    async fn tx_delete_by_id(&self, _tx_id: &str, _workspace: &str, _id: &str) -> Result<()> {
        Ok(())
    }

    async fn tx_get(&self, _tx_id: &str, workspace: &str, id: &str) -> Result<Option<Value>> {
        Ok(Some(mock_node(workspace, "/mock-path", id, "raisin:Page")))
    }

    async fn tx_get_by_path(
        &self,
        _tx_id: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>> {
        Ok(Some(mock_node(workspace, path, "mock-id", "raisin:Page")))
    }

    async fn tx_list_children(
        &self,
        _tx_id: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Value>> {
        Ok(mock_children(workspace, parent_path, 2))
    }

    async fn tx_move(
        &self,
        _tx_id: &str,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        Ok(mock_moved_node(workspace, node_path, new_parent_path))
    }

    async fn tx_update_property(
        &self,
        _tx_id: &str,
        _workspace: &str,
        _node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        tracing::info!(property_path = %property_path, value = ?value, "Mock tx property update");
        Ok(())
    }

    // ========== Date/Time Operations ==========

    fn date_now(&self) -> String {
        Utc::now().to_rfc3339()
    }
    fn date_timestamp(&self) -> i64 {
        Utc::now().timestamp()
    }
    fn date_timestamp_millis(&self) -> i64 {
        Utc::now().timestamp_millis()
    }

    fn date_parse(&self, date_str: &str, format: Option<&str>) -> Result<i64> {
        let dt = match format {
            Some(fmt) => {
                let naive = NaiveDateTime::parse_from_str(date_str, fmt).map_err(|e| {
                    raisin_error::Error::Validation(format!("Invalid date format: {}", e))
                })?;
                Utc.from_utc_datetime(&naive)
            }
            None => DateTime::parse_from_rfc3339(date_str)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    NaiveDateTime::parse_from_str(
                        &format!("{}T00:00:00", date_str),
                        "%Y-%m-%dT%H:%M:%S",
                    )
                    .map(|naive| Utc.from_utc_datetime(&naive))
                })
                .map_err(|e| raisin_error::Error::Validation(format!("Invalid ISO date: {}", e)))?,
        };
        Ok(dt.timestamp())
    }

    fn date_format(&self, timestamp: i64, format: Option<&str>) -> Result<String> {
        let dt = Utc
            .timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| raisin_error::Error::Validation("Invalid timestamp".to_string()))?;
        Ok(dt
            .format(format.unwrap_or("%Y-%m-%dT%H:%M:%SZ"))
            .to_string())
    }

    fn date_add_days(&self, timestamp: i64, days: i64) -> Result<i64> {
        let dt = Utc
            .timestamp_opt(timestamp, 0)
            .single()
            .ok_or_else(|| raisin_error::Error::Validation("Invalid timestamp".to_string()))?;
        Ok((dt + Duration::days(days)).timestamp())
    }

    fn date_diff_days(&self, ts1: i64, ts2: i64) -> i64 {
        (ts2 - ts1) / 86400
    }

    fn log(&self, level: &str, message: &str) {
        self.logs
            .lock()
            .unwrap()
            .push((level.to_string(), message.to_string()));
        match level {
            "debug" => tracing::debug!("{}", message),
            "info" => tracing::info!("{}", message),
            "warn" => tracing::warn!("{}", message),
            "error" => tracing::error!("{}", message),
            _ => tracing::info!("{}", message),
        }
    }

    fn get_context(&self) -> Value {
        self.context.clone()
    }
    fn allows_admin_escalation(&self) -> bool {
        true
    }

    // ========== Admin Operations (delegate to non-admin) ==========

    async fn admin_node_get(&self, ws: &str, path: &str) -> Result<Option<Value>> {
        self.node_get(ws, path).await
    }
    async fn admin_node_get_by_id(&self, ws: &str, id: &str) -> Result<Option<Value>> {
        self.node_get_by_id(ws, id).await
    }
    async fn admin_node_create(&self, ws: &str, pp: &str, data: Value) -> Result<Value> {
        self.node_create(ws, pp, data).await
    }
    async fn admin_node_update(&self, ws: &str, path: &str, data: Value) -> Result<Value> {
        self.node_update(ws, path, data).await
    }
    async fn admin_node_delete(&self, ws: &str, path: &str) -> Result<()> {
        self.node_delete(ws, path).await
    }
    async fn admin_node_update_property(
        &self,
        ws: &str,
        np: &str,
        pp: &str,
        v: Value,
    ) -> Result<()> {
        self.node_update_property(ws, np, pp, v).await
    }
    async fn admin_node_query(&self, ws: &str, q: Value) -> Result<Vec<Value>> {
        self.node_query(ws, q).await
    }
    async fn admin_node_get_children(
        &self,
        ws: &str,
        pp: &str,
        l: Option<u32>,
    ) -> Result<Vec<Value>> {
        self.node_get_children(ws, pp, l).await
    }
    async fn admin_sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value> {
        self.sql_query(sql, params).await
    }
    async fn admin_sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64> {
        self.sql_execute(sql, params).await
    }

    // ========== Lock / Inventory Operations ==========

    async fn lock_acquire(
        &self,
        key: &str,
        ttl_ms: i64,
        owner: Option<String>,
    ) -> Result<Option<Value>> {
        use raisin_locks::LockManager;
        let owner = owner.unwrap_or_else(|| "mock".to_string());
        let ttl = std::time::Duration::from_millis(ttl_ms.max(0) as u64);
        let guard = self.locks.try_acquire(key, &owner, ttl).await?;
        Ok(guard.map(|g| serde_json::to_value(g).unwrap_or(Value::Null)))
    }

    async fn lock_release(&self, key: &str, token: i64) -> Result<bool> {
        use raisin_locks::LockManager;
        self.locks.release(key, token as u64).await
    }

    async fn lock_renew(&self, key: &str, token: i64, ttl_ms: i64) -> Result<bool> {
        use raisin_locks::LockManager;
        let ttl = std::time::Duration::from_millis(ttl_ms.max(0) as u64);
        self.locks.renew(key, token as u64, ttl).await
    }

    async fn inventory_claim(&self, pool: &str, n: i64, capacity: i64) -> Result<Option<Value>> {
        use raisin_locks::LockManager;
        let remaining = self
            .locks
            .claim(pool, n.max(0) as u64, capacity.max(0) as u64)
            .await?;
        Ok(remaining.map(|r| serde_json::json!({ "remaining": r })))
    }

    async fn inventory_release(&self, pool: &str, n: i64) -> Result<i64> {
        use raisin_locks::LockManager;
        let remaining = self.locks.release_claim(pool, n.max(0) as u64).await?;
        Ok(remaining as i64)
    }

    // ========== Secrets (in-memory, append-only) ==========
    //
    // NOTE: the mock enforces NO SecretPolicy. The policy gate lives in
    // `RaisinFunctionApi` (api/raisindb/secrets.rs) and is tested there; these
    // fixtures exist so the runtime bindings can be exercised without a
    // keyring or a RocksDB handle.

    async fn secret_get(&self, spec: &str, version: Option<i64>) -> Result<String> {
        // Same parse the real API does, via the same helper: the mock must
        // accept a `secret://name@N` reference or the runtime parity tests
        // would pass while the real surface differs.
        let parsed = crate::api::parse_read_spec(spec, version)?;
        let name = parsed.name.as_str();
        let store = self.secrets.lock().unwrap();
        let versions = store.get(name).ok_or_else(|| {
            raisin_error::Error::NotFound(format!("secret '{name}' is not present on this node"))
        })?;
        // Ordinals are 1-based, matching SecretRecord::version.
        let idx = match parsed.version {
            Some(v) if v >= 1 => (v as usize) - 1,
            _ => versions.len().saturating_sub(1),
        };
        match versions.get(idx) {
            Some(Some(value)) => Ok(value.clone()),
            Some(None) => Err(raisin_error::Error::NotFound(format!(
                "secret '{name}' version {} was deleted",
                idx + 1
            ))),
            None => Err(raisin_error::Error::NotFound(format!(
                "secret '{name}' version {} is not present on this node",
                idx + 1
            ))),
        }
    }

    async fn secret_resolve(&self, value: &str) -> Result<String> {
        if !raisin_models::secret_ref::SecretRef::is_secret_ref(value) {
            return Ok(value.to_string());
        }
        self.secret_get(value, None).await
    }

    async fn secret_put(&self, spec: &str, value: &str) -> Result<Value> {
        let name = crate::api::parse_write_spec(spec, "write")?;
        let name = name.as_str();
        let mut store = self.secrets.lock().unwrap();
        let versions = store.entry(name.to_string()).or_default();
        versions.push(Some(value.to_string()));
        Ok(serde_json::json!({ "name": name, "version": versions.len() }))
    }

    async fn secret_list(&self) -> Result<Vec<Value>> {
        let store = self.secrets.lock().unwrap();
        let mut names: Vec<&String> = store.keys().collect();
        names.sort(); // deterministic for tests
        Ok(names
            .into_iter()
            .map(|name| {
                let versions = &store[name];
                serde_json::json!({
                    "name": name,
                    "version": versions.len(),
                    "deleted": versions.last().map(|v| v.is_none()).unwrap_or(false),
                    "created_by": "mock",
                })
            })
            .collect())
    }

    async fn secret_rotate(&self, spec: &str, value: &str) -> Result<Value> {
        self.secret_put(spec, value).await
    }

    async fn secret_delete(&self, spec: &str) -> Result<Value> {
        let name = crate::api::parse_write_spec(spec, "delete")?;
        let name = name.as_str();
        let mut store = self.secrets.lock().unwrap();
        let versions = store.entry(name.to_string()).or_default();
        versions.push(None);
        Ok(serde_json::json!({ "name": name, "version": versions.len() }))
    }

    async fn integrations_sync_now(&self, _mount_id: &str, _mode: Option<&str>) -> Result<Value> {
        // No job queue in the mock: report the request as freshly queued with a
        // null job id, matching the real "already deduped" shape.
        Ok(serde_json::json!({ "job_id": Value::Null, "status": "queued" }))
    }

    // ========== IMAP (deterministic in-memory fixtures) ==========

    async fn imap_fetch_since(
        &self,
        _conn: Value,
        since_uid: i64,
        opts: Option<Value>,
    ) -> Result<Value> {
        let since = since_uid.max(0) as u32;
        let limit = opts
            .as_ref()
            .and_then(|o| o.get("limit"))
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(200);

        // Two fixed messages so downstream + wrapper tests are deterministic.
        let all = mock_imap_messages();

        let mut messages: Vec<Value> = all
            .into_iter()
            .filter(|(uid, _, _)| *uid > since)
            .map(|(uid, from, subject)| {
                serde_json::json!({
                    "uid": uid,
                    "headers": { "from": from, "subject": subject },
                    "from": from,
                    "to": "inbox@example.org",
                    "subject": subject,
                    "date": "Mon, 01 Jan 2024 00:00:00 +0000",
                    "snippet": "",
                    "flags": ["\\Seen"],
                    "message_id": format!("<{uid}@mock.example.org>"),
                })
            })
            .collect();
        if messages.len() > limit {
            messages.truncate(limit);
        }

        // Never emit a null cursor: when nothing is new, echo the input cursor.
        let highest = messages
            .iter()
            .filter_map(|m| m.get("uid").and_then(|v| v.as_u64()))
            .map(|u| u as u32)
            .max()
            .unwrap_or(since);

        Ok(serde_json::json!({
            "messages": messages,
            "highestUid": highest,
            "uidvalidity": 1u32,
        }))
    }

    async fn imap_list_mailboxes(&self, _conn: Value) -> Result<Value> {
        Ok(serde_json::json!([
            { "name": "INBOX", "path": "INBOX", "flags": ["\\HasNoChildren"] },
            { "name": "Sent", "path": "Sent", "flags": ["\\HasNoChildren"] },
        ]))
    }

    async fn imap_fetch_message(
        &self,
        _conn: Value,
        uid: i64,
        _opts: Option<Value>,
    ) -> Result<Value> {
        let uid = uid.max(0) as u32;
        Ok(serde_json::json!({
            "uid": uid,
            "headers": { "subject": format!("Mock message {uid}") },
            "from": "alice@example.org",
            "to": "inbox@example.org",
            "subject": format!("Mock message {uid}"),
            "date": "Mon, 01 Jan 2024 00:00:00 +0000",
            "text": format!("Body of message {uid}."),
            "snippet": format!("Body of message {uid}."),
            "flags": ["\\Seen"],
            "message_id": format!("<{uid}@mock.example.org>"),
        }))
    }

    // ========== Email (recorded, never sent) ==========

    // ========== Identities ==========
    //
    // NOTE: the mock enforces NO IdentityPolicy. The gate lives in
    // `RaisinFunctionApi` (api/raisindb/identities.rs) and is tested there.

    async fn identity_find_by_email(&self, _email: &str) -> Result<Option<Value>> {
        self.check_all_errors()?;
        Ok(None)
    }

    async fn identity_update(&self, id: &str, patch: Value) -> Result<Value> {
        self.check_all_errors()?;
        // Echo the patch in the public shape; never the password itself.
        Ok(serde_json::json!({
            "id": id,
            "email": patch.get("email").cloned().unwrap_or(Value::Null),
            "email_verified": false,
            "display_name": patch.get("display_name").cloned().unwrap_or(Value::Null),
            "has_password": patch.get("password").is_some(),
        }))
    }

    async fn email_send(&self, message: Value) -> Result<Value> {
        self.check_all_errors()?;
        let mut sent = self.emails_sent.lock().unwrap();
        let named = message
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();
        sent.push(message);
        // Sequence-numbered so a test asserting on the second send does not
        // depend on the first, and obviously fake so a mock id can never be
        // mistaken for a provider one.
        // `named` is captured above, before the message is recorded: a test can
        // then assert that a function routed a send to the account it meant to.
        Ok(serde_json::json!({
            "message_id": format!("mock-email-{}", sent.len()),
            "provider": "resend",
            "sender": named,
        }))
    }

    async fn email_providers(&self) -> Result<Value> {
        self.check_all_errors()?;
        Ok(serde_json::json!({
            "enabled": true,
            "providers": [{
                "name": "default",
                "provider": "resend",
                "from_address": "noreply@example.test",
                "enabled": true,
                "default": true,
            }],
        }))
    }

    // ========== Crypto (deterministic) ==========

    async fn crypto_verify_jwt(&self, _token: &str, _opts: Value) -> Result<Value> {
        // Deterministic success so round-trip/wrapper tests don't need a real
        // JWKS or network. Real verification lives in `runtime::crypto`.
        Ok(serde_json::json!({ "valid": true, "claims": {} }))
    }

    async fn crypto_random_bytes(&self, n: u32) -> Result<String> {
        // Deterministic and obviously fake: a repeating 'A' pattern can never
        // be mistaken for entropy if it escapes into a fixture or a snapshot.
        // Only the LENGTH contract is honoured (base64url of `n` bytes).
        //
        // THE BOUND IS ENFORCED HERE TOO. `ArgParser::u32` casts with `as u32`,
        // so `randomBytes(-1)` reaches this method as 4294967295 — and a mock
        // that trusts `n` allocates 4 GiB before anything can object. The real
        // implementation rejects it; a mock that does not is a way to fell the
        // test harness with a plausible-looking call.
        use base64::Engine as _;
        crate::runtime::crypto::random_bytes_check_len(n)?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(vec![0x41u8; n as usize]))
    }

    async fn crypto_hash(&self, input: &str, alg: Option<&str>) -> Result<String> {
        // The real digest, not a stub: tests that assert a known-answer digest
        // are the whole point of the binding, and a fake here would hide a
        // wiring bug behind a passing test.
        crate::runtime::crypto::hash_hex(input, alg)
    }

    async fn crypto_generate_key_pair(&self, alg: Option<&str>) -> Result<Value> {
        // Real keygen: it is pure, offline and fast, and a fixed fake keypair
        // in a mock is exactly the kind of thing that ends up in production.
        crate::runtime::crypto::generate_key_pair(alg)
    }

    async fn crypto_sign_jwt(
        &self,
        claims: Value,
        private_jwk: Value,
        opts: Value,
    ) -> Result<String> {
        crate::runtime::crypto::sign_jwt(
            &claims,
            &private_jwk,
            &crate::runtime::crypto::SignJwtOptions::from_value(&opts)?,
        )
    }
}

/// Deterministic mock mailbox: `(uid, from, subject)` tuples.
fn mock_imap_messages() -> Vec<(u32, &'static str, &'static str)> {
    vec![
        (101, "alice@example.org", "Mock One"),
        (102, "bob@example.org", "Mock Two"),
    ]
}
