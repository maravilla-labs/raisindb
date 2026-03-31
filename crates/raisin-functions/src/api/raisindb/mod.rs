// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! RaisinDB FunctionApi implementation
//!
//! This module provides the real FunctionApi implementation that connects
//! to RaisinDB backend services through callbacks. The callbacks are provided
//! by the transport layer which has access to node services, SQL engine, etc.
//!
//! The implementation is split into submodules by operation category:
//! - `nodes` - Node CRUD and query operations
//! - `sql` - SQL query and execute operations
//! - `ai` - AI completion, models, and embedding operations
//! - `tasks` - Human task lifecycle operations
//! - `tx` - Transaction operations
//! - `http` - HTTP request operations with network policy
//! - `events` - Event emission operations
//! - `resources` - Binary resource and PDF operations
//! - `admin` - Admin-escalated node and SQL operations
//! - `context` - Date/time, logging, and execution context
//! - `functions` - Function-to-function call operations

mod admin;
mod ai_ops;
mod context;
mod events;
mod functions;
mod http;
mod network_policy;
mod nodes;
mod resources;
mod sql;
mod tasks;
mod tx;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use serde_json::Value;
use std::sync::{Arc, Mutex};

use super::callbacks::RaisinFunctionApiCallbacks;
use super::FunctionApi;
use crate::types::{ExecutionContext, LogEntry, NetworkPolicy};

/// Real FunctionApi implementation for RaisinDB
///
/// This implementation uses callbacks provided by the transport layer
/// to perform actual operations against RaisinDB services.
pub struct RaisinFunctionApi {
    /// Execution context
    pub(crate) context: ExecutionContext,
    /// Network policy for HTTP access control
    pub(crate) network_policy: NetworkPolicy,
    /// Callbacks for operations
    pub(crate) callbacks: RaisinFunctionApiCallbacks,
    /// Captured logs
    pub(crate) logs: Arc<Mutex<Vec<LogEntry>>>,
}

impl RaisinFunctionApi {
    /// Create a new RaisinFunctionApi
    pub fn new(
        context: ExecutionContext,
        network_policy: NetworkPolicy,
        callbacks: RaisinFunctionApiCallbacks,
    ) -> Self {
        tracing::trace!(
            http_enabled = network_policy.http_enabled,
            allowed_urls = ?network_policy.allowed_urls,
            "RaisinFunctionApi::new"
        );
        Self {
            context,
            network_policy,
            callbacks,
            logs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Get captured logs
    pub fn get_logs(&self) -> Vec<LogEntry> {
        self.logs.lock().unwrap().clone()
    }

    /// Get the execution context
    pub fn context(&self) -> &ExecutionContext {
        &self.context
    }

    /// Get the auth context (if set)
    pub fn auth_context(&self) -> Option<&AuthContext> {
        self.context.auth_context.as_ref()
    }

    /// Check if this function is allowed to escalate to admin context
    pub fn allows_admin_escalation(&self) -> bool {
        self.context.allows_admin_escalation
    }
}

#[async_trait]
impl FunctionApi for RaisinFunctionApi {
    // ========== Node Operations ==========

    async fn node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>> {
        self.impl_node_get(workspace, path).await
    }

    async fn node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>> {
        self.impl_node_get_by_id(workspace, id).await
    }

    async fn node_create(&self, workspace: &str, parent_path: &str, data: Value) -> Result<Value> {
        self.impl_node_create(workspace, parent_path, data).await
    }

    async fn node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value> {
        self.impl_node_update(workspace, path, data).await
    }

    async fn node_delete(&self, workspace: &str, path: &str) -> Result<()> {
        self.impl_node_delete(workspace, path).await
    }

    async fn node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        self.impl_node_update_property(workspace, node_path, property_path, value)
            .await
    }

    async fn node_move(
        &self,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        self.impl_node_move(workspace, node_path, new_parent_path)
            .await
    }

    async fn node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>> {
        self.impl_node_query(workspace, query).await
    }

    async fn node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        self.impl_node_get_children(workspace, parent_path, limit)
            .await
    }

    // ========== SQL Operations ==========

    async fn sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value> {
        self.impl_sql_query(sql, params).await
    }

    async fn sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64> {
        self.impl_sql_execute(sql, params).await
    }

    // ========== HTTP Operations ==========

    async fn http_request(&self, method: &str, url: &str, options: Value) -> Result<Value> {
        self.impl_http_request(method, url, options).await
    }

    // ========== Event Operations ==========

    async fn emit_event(&self, event_type: &str, data: Value) -> Result<()> {
        self.impl_emit_event(event_type, data).await
    }

    // ========== AI Operations ==========

    async fn ai_completion(&self, request: Value) -> Result<Value> {
        self.impl_ai_completion(request).await
    }

    async fn ai_list_models(&self) -> Result<Vec<Value>> {
        self.impl_ai_list_models().await
    }

    async fn ai_get_default_model(&self, use_case: &str) -> Result<Option<String>> {
        self.impl_ai_get_default_model(use_case).await
    }

    async fn ai_embed(&self, request: Value) -> Result<Value> {
        self.impl_ai_embed(request).await
    }

    // ========== Resource Operations ==========

    async fn resource_get_binary(&self, storage_key: &str) -> Result<String> {
        self.impl_resource_get_binary(storage_key).await
    }

    async fn node_add_resource(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        upload_data: Value,
    ) -> Result<Value> {
        self.impl_node_add_resource(workspace, node_path, property_path, upload_data)
            .await
    }

    async fn pdf_process_from_storage(&self, storage_key: &str, options: Value) -> Result<Value> {
        self.impl_pdf_process_from_storage(storage_key, options)
            .await
    }

    // ========== Task Operations ==========

    async fn task_create(&self, request: Value) -> Result<Value> {
        self.impl_task_create(request).await
    }

    async fn task_update(&self, task_id: &str, updates: Value) -> Result<Value> {
        self.impl_task_update(task_id, updates).await
    }

    async fn task_complete(&self, task_id: &str, response: Value) -> Result<Value> {
        self.impl_task_complete(task_id, response).await
    }

    async fn task_query(&self, query: Value) -> Result<Vec<Value>> {
        self.impl_task_query(query).await
    }

    // ========== Function Operations ==========

    async fn function_execute(
        &self,
        function_path: &str,
        arguments: Value,
        context: super::FunctionExecuteContext,
    ) -> Result<Value> {
        self.impl_function_execute(function_path, arguments, context)
            .await
    }

    async fn function_call(&self, function_path: &str, arguments: Value) -> Result<Value> {
        self.impl_function_call(function_path, arguments).await
    }

    // ========== Date/Time Operations ==========

    fn date_now(&self) -> String {
        self.impl_date_now()
    }

    fn date_timestamp(&self) -> i64 {
        self.impl_date_timestamp()
    }

    fn date_timestamp_millis(&self) -> i64 {
        self.impl_date_timestamp_millis()
    }

    fn date_parse(&self, date_str: &str, format: Option<&str>) -> Result<i64> {
        self.impl_date_parse(date_str, format)
    }

    fn date_format(&self, timestamp: i64, format: Option<&str>) -> Result<String> {
        self.impl_date_format(timestamp, format)
    }

    fn date_add_days(&self, timestamp: i64, days: i64) -> Result<i64> {
        self.impl_date_add_days(timestamp, days)
    }

    fn date_diff_days(&self, ts1: i64, ts2: i64) -> i64 {
        self.impl_date_diff_days(ts1, ts2)
    }

    fn log(&self, level: &str, message: &str) {
        self.impl_log(level, message);
    }

    fn get_context(&self) -> Value {
        self.impl_get_context()
    }

    // ========== Transaction Operations ==========

    async fn tx_begin(&self) -> Result<String> {
        self.impl_tx_begin().await
    }

    async fn tx_commit(&self, tx_id: &str) -> Result<()> {
        self.impl_tx_commit(tx_id).await
    }

    async fn tx_rollback(&self, tx_id: &str) -> Result<()> {
        self.impl_tx_rollback(tx_id).await
    }

    async fn tx_set_actor(&self, tx_id: &str, actor: &str) -> Result<()> {
        self.impl_tx_set_actor(tx_id, actor).await
    }

    async fn tx_set_message(&self, tx_id: &str, message: &str) -> Result<()> {
        self.impl_tx_set_message(tx_id, message).await
    }

    async fn tx_create(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value> {
        self.impl_tx_create(tx_id, workspace, parent_path, data)
            .await
    }

    async fn tx_add(&self, tx_id: &str, workspace: &str, data: Value) -> Result<Value> {
        self.impl_tx_add(tx_id, workspace, data).await
    }

    async fn tx_put(&self, tx_id: &str, workspace: &str, data: Value) -> Result<()> {
        self.impl_tx_put(tx_id, workspace, data).await
    }

    async fn tx_upsert(&self, tx_id: &str, workspace: &str, data: Value) -> Result<()> {
        self.impl_tx_upsert(tx_id, workspace, data).await
    }

    async fn tx_create_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<Value> {
        self.impl_tx_create_deep(tx_id, workspace, parent_path, data, parent_node_type)
            .await
    }

    async fn tx_upsert_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<()> {
        self.impl_tx_upsert_deep(tx_id, workspace, data, parent_node_type)
            .await
    }

    async fn tx_update(&self, tx_id: &str, workspace: &str, path: &str, data: Value) -> Result<()> {
        self.impl_tx_update(tx_id, workspace, path, data).await
    }

    async fn tx_delete(&self, tx_id: &str, workspace: &str, path: &str) -> Result<()> {
        self.impl_tx_delete(tx_id, workspace, path).await
    }

    async fn tx_delete_by_id(&self, tx_id: &str, workspace: &str, id: &str) -> Result<()> {
        self.impl_tx_delete_by_id(tx_id, workspace, id).await
    }

    async fn tx_get(&self, tx_id: &str, workspace: &str, id: &str) -> Result<Option<Value>> {
        self.impl_tx_get(tx_id, workspace, id).await
    }

    async fn tx_get_by_path(
        &self,
        tx_id: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>> {
        self.impl_tx_get_by_path(tx_id, workspace, path).await
    }

    async fn tx_list_children(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Value>> {
        self.impl_tx_list_children(tx_id, workspace, parent_path)
            .await
    }

    async fn tx_move(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        self.impl_tx_move(tx_id, workspace, node_path, new_parent_path)
            .await
    }

    async fn tx_update_property(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        self.impl_tx_update_property(tx_id, workspace, node_path, property_path, value)
            .await
    }

    // ========== Admin Escalation ==========

    fn allows_admin_escalation(&self) -> bool {
        self.context.allows_admin_escalation
    }

    // ========== Admin Node Operations ==========

    async fn admin_node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>> {
        self.impl_admin_node_get(workspace, path).await
    }

    async fn admin_node_get_by_id(&self, workspace: &str, id: &str) -> Result<Option<Value>> {
        self.impl_admin_node_get_by_id(workspace, id).await
    }

    async fn admin_node_create(
        &self,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value> {
        self.impl_admin_node_create(workspace, parent_path, data)
            .await
    }

    async fn admin_node_update(&self, workspace: &str, path: &str, data: Value) -> Result<Value> {
        self.impl_admin_node_update(workspace, path, data).await
    }

    async fn admin_node_delete(&self, workspace: &str, path: &str) -> Result<()> {
        self.impl_admin_node_delete(workspace, path).await
    }

    async fn admin_node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        self.impl_admin_node_update_property(workspace, node_path, property_path, value)
            .await
    }

    async fn admin_node_query(&self, workspace: &str, query: Value) -> Result<Vec<Value>> {
        self.impl_admin_node_query(workspace, query).await
    }

    async fn admin_node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        self.impl_admin_node_get_children(workspace, parent_path, limit)
            .await
    }

    // ========== Admin SQL Operations ==========

    async fn admin_sql_query(&self, sql: &str, params: Vec<Value>) -> Result<Value> {
        self.impl_admin_sql_query(sql, params).await
    }

    async fn admin_sql_execute(&self, sql: &str, params: Vec<Value>) -> Result<i64> {
        self.impl_admin_sql_execute(sql, params).await
    }
}
