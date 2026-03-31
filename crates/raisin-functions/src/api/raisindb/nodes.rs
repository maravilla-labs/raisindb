// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Node operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_node_get(&self, workspace: &str, path: &str) -> Result<Option<Value>> {
        let callback = self.callbacks.node_get.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node get callback not configured".to_string())
        })?;

        callback(workspace.to_string(), path.to_string()).await
    }

    pub(crate) async fn impl_node_get_by_id(
        &self,
        workspace: &str,
        id: &str,
    ) -> Result<Option<Value>> {
        let callback = self.callbacks.node_get_by_id.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node get by ID callback not configured".to_string())
        })?;

        callback(workspace.to_string(), id.to_string()).await
    }

    pub(crate) async fn impl_node_create(
        &self,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.node_create.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node create callback not configured".to_string())
        })?;

        callback(workspace.to_string(), parent_path.to_string(), data).await
    }

    pub(crate) async fn impl_node_update(
        &self,
        workspace: &str,
        path: &str,
        data: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.node_update.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node update callback not configured".to_string())
        })?;

        callback(workspace.to_string(), path.to_string(), data).await
    }

    pub(crate) async fn impl_node_delete(&self, workspace: &str, path: &str) -> Result<()> {
        let callback = self.callbacks.node_delete.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node delete callback not configured".to_string())
        })?;

        callback(workspace.to_string(), path.to_string()).await
    }

    pub(crate) async fn impl_node_update_property(
        &self,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        let callback = self
            .callbacks
            .node_update_property
            .as_ref()
            .ok_or_else(|| {
                raisin_error::Error::Validation(
                    "Node update property callback not configured".to_string(),
                )
            })?;

        callback(
            workspace.to_string(),
            node_path.to_string(),
            property_path.to_string(),
            value,
        )
        .await
    }

    pub(crate) async fn impl_node_move(
        &self,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        let callback = self.callbacks.node_move.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node move callback not configured".to_string())
        })?;

        callback(
            workspace.to_string(),
            node_path.to_string(),
            new_parent_path.to_string(),
        )
        .await
    }

    pub(crate) async fn impl_node_query(
        &self,
        workspace: &str,
        query: Value,
    ) -> Result<Vec<Value>> {
        let callback = self.callbacks.node_query.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node query callback not configured".to_string())
        })?;

        callback(workspace.to_string(), query).await
    }

    pub(crate) async fn impl_node_get_children(
        &self,
        workspace: &str,
        parent_path: &str,
        limit: Option<u32>,
    ) -> Result<Vec<Value>> {
        let callback = self.callbacks.node_get_children.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Node get children callback not configured".to_string())
        })?;

        callback(workspace.to_string(), parent_path.to_string(), limit).await
    }
}
