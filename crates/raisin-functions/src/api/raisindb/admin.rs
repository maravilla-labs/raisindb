// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Admin-escalated node and SQL operation implementations for RaisinFunctionApi
//!
//! TODO: Once admin callbacks are added to RaisinFunctionApiCallbacks,
//! these should use dedicated admin callbacks that bypass RLS.
//! For now, they call the regular callbacks which will use
//! the auth context (or None for system context).

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    // ========== Admin Node Operations ==========

    pub(crate) async fn impl_admin_node_get(
        &self,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>> {
        // For now, delegate to regular callback
        // TODO: Use admin_node_get callback when available
        self.impl_node_get(workspace, path).await
    }

    pub(crate) async fn impl_admin_node_get_by_id(
        &self,
        workspace: &str,
        id: &str,
    ) -> Result<Option<Value>> {
        self.impl_node_get_by_id(workspace, id).await
    }

    pub(crate) async fn impl_admin_node_create(
        &self,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value> {
        self.impl_node_create(workspace, parent_path, data).await
    }

    pub(crate) async fn impl_admin_node_update(
        &self,
        workspace: &str,
        path: &str,
        data: Value,
    ) -> Result<Value> {
        self.impl_node_update(workspace, path, data).await
    }

    pub(crate) async fn impl_admin_node_delete(&self, workspace: &str, path: &str) -> Result<()> {
        self.impl_node_delete(workspace, path).await
    }

    pub(crate) async fn impl_admin_node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        self.impl_node_update_property(workspace, node_path, property_path, value)
            .await
    }

    pub(crate) async fn impl_admin_node_query(
        &self,
        workspace: &str,
        query: Value,
    ) -> Result<Vec<Value>> {
        self.impl_node_query(workspace, query).await
    }

    pub(crate) async fn impl_admin_node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        self.impl_node_get_children(workspace, parent_path, limit)
            .await
    }

    // ========== Admin SQL Operations ==========

    pub(crate) async fn impl_admin_sql_query(
        &self,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<Value> {
        self.impl_sql_query(sql, params).await
    }

    pub(crate) async fn impl_admin_sql_execute(
        &self,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<i64> {
        self.impl_sql_execute(sql, params).await
    }
}
