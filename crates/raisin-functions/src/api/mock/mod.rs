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
}

impl MockFunctionApi {
    /// Create a new mock API
    pub fn new(context: Value) -> Self {
        Self {
            context,
            logs: std::sync::Mutex::new(Vec::new()),
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
        Ok(Some(mock_node(
            workspace,
            path,
            "mock-node-id",
            "raisin:Page",
        )))
    }

    async fn node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>> {
        Ok(Some(mock_node(workspace, "/mock-path", id, "raisin:Page")))
    }

    async fn node_create(&self, workspace: &str, parent_path: &str, data: Value) -> Result<Value> {
        Ok(mock_created_node(workspace, parent_path, &data))
    }

    async fn node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value> {
        Ok(mock_updated_node(workspace, path, &data))
    }

    async fn node_delete(&self, _workspace: &str, _path: &str) -> Result<()> {
        Ok(())
    }

    async fn node_update_property(
        &self,
        _workspace: &str,
        _node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        tracing::info!(property_path = %property_path, value = ?value, "Mock property update");
        Ok(())
    }

    async fn node_move(
        &self,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        Ok(mock_moved_node(workspace, node_path, new_parent_path))
    }

    async fn node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>> {
        let limit = query.get("limit").and_then(|l| l.as_u64()).unwrap_or(10) as usize;
        Ok(mock_query_results(workspace, &query, limit))
    }

    async fn node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        let count = limit.unwrap_or(3) as usize;
        Ok(mock_children(workspace, parent_path, count))
    }

    // ========== SQL Operations ==========

    async fn sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value> {
        Ok(serde_json::json!({
            "columns": ["id", "name"], "rows": [["1", "test"]], "row_count": 1,
            "_debug": { "sql": sql, "params": params }
        }))
    }

    async fn sql_execute(&self, _sql: &str, _params: Vec<Value>) -> Result<i64> {
        Ok(1)
    }

    // ========== HTTP / Event Operations ==========

    async fn http_request(&self, method: &str, url: &str, options: Value) -> Result<Value> {
        Ok(serde_json::json!({
            "status": 200, "headers": {},
            "body": { "_mock": true, "method": method, "url": url, "options": options }
        }))
    }

    async fn emit_event(&self, event_type: &str, data: Value) -> Result<()> {
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
}
