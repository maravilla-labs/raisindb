// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Transaction operation implementations for RaisinFunctionApi

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    pub(crate) async fn impl_tx_begin(&self) -> Result<String> {
        let callback = self.callbacks.tx_begin.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Transaction begin callback not configured".to_string())
        })?;
        callback().await
    }

    pub(crate) async fn impl_tx_commit(&self, tx_id: &str) -> Result<()> {
        let callback = self.callbacks.tx_commit.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction commit callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string()).await
    }

    pub(crate) async fn impl_tx_rollback(&self, tx_id: &str) -> Result<()> {
        let callback = self.callbacks.tx_rollback.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction rollback callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string()).await
    }

    pub(crate) async fn impl_tx_set_actor(&self, tx_id: &str, actor: &str) -> Result<()> {
        let callback = self.callbacks.tx_set_actor.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction set actor callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string(), actor.to_string()).await
    }

    pub(crate) async fn impl_tx_set_message(&self, tx_id: &str, message: &str) -> Result<()> {
        let callback = self.callbacks.tx_set_message.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction set message callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string(), message.to_string()).await
    }

    pub(crate) async fn impl_tx_create(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.tx_create.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction create callback not configured".to_string(),
            )
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            parent_path.to_string(),
            data,
        )
        .await
    }

    pub(crate) async fn impl_tx_add(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
    ) -> Result<Value> {
        let callback = self.callbacks.tx_add.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Transaction add callback not configured".to_string())
        })?;
        callback(tx_id.to_string(), workspace.to_string(), data).await
    }

    pub(crate) async fn impl_tx_put(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
    ) -> Result<()> {
        let callback = self.callbacks.tx_put.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Transaction put callback not configured".to_string())
        })?;
        callback(tx_id.to_string(), workspace.to_string(), data).await
    }

    pub(crate) async fn impl_tx_upsert(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
    ) -> Result<()> {
        let callback = self.callbacks.tx_upsert.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction upsert callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string(), workspace.to_string(), data).await
    }

    pub(crate) async fn impl_tx_create_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<Value> {
        let callback = self.callbacks.tx_create_deep.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction create deep callback not configured".to_string(),
            )
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            parent_path.to_string(),
            data,
            parent_node_type.to_string(),
        )
        .await
    }

    pub(crate) async fn impl_tx_upsert_deep(
        &self,
        tx_id: &str,
        workspace: &str,
        data: Value,
        parent_node_type: &str,
    ) -> Result<()> {
        let callback = self.callbacks.tx_upsert_deep.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction upsert deep callback not configured".to_string(),
            )
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            data,
            parent_node_type.to_string(),
        )
        .await
    }

    pub(crate) async fn impl_tx_update(
        &self,
        tx_id: &str,
        workspace: &str,
        path: &str,
        data: Value,
    ) -> Result<()> {
        let callback = self.callbacks.tx_update.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction update callback not configured".to_string(),
            )
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            path.to_string(),
            data,
        )
        .await
    }

    pub(crate) async fn impl_tx_delete(
        &self,
        tx_id: &str,
        workspace: &str,
        path: &str,
    ) -> Result<()> {
        let callback = self.callbacks.tx_delete.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction delete callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string(), workspace.to_string(), path.to_string()).await
    }

    pub(crate) async fn impl_tx_delete_by_id(
        &self,
        tx_id: &str,
        workspace: &str,
        id: &str,
    ) -> Result<()> {
        let callback = self.callbacks.tx_delete_by_id.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction delete by ID callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string(), workspace.to_string(), id.to_string()).await
    }

    pub(crate) async fn impl_tx_get(
        &self,
        tx_id: &str,
        workspace: &str,
        id: &str,
    ) -> Result<Option<Value>> {
        let callback = self.callbacks.tx_get.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Transaction get callback not configured".to_string())
        })?;
        callback(tx_id.to_string(), workspace.to_string(), id.to_string()).await
    }

    pub(crate) async fn impl_tx_get_by_path(
        &self,
        tx_id: &str,
        workspace: &str,
        path: &str,
    ) -> Result<Option<Value>> {
        let callback = self.callbacks.tx_get_by_path.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction get by path callback not configured".to_string(),
            )
        })?;
        callback(tx_id.to_string(), workspace.to_string(), path.to_string()).await
    }

    pub(crate) async fn impl_tx_list_children(
        &self,
        tx_id: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Vec<Value>> {
        let callback = self.callbacks.tx_list_children.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction list children callback not configured".to_string(),
            )
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            parent_path.to_string(),
        )
        .await
    }

    pub(crate) async fn impl_tx_move(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        new_parent_path: &str,
    ) -> Result<Value> {
        let callback = self.callbacks.tx_move.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("Transaction move callback not configured".to_string())
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            node_path.to_string(),
            new_parent_path.to_string(),
        )
        .await
    }

    pub(crate) async fn impl_tx_update_property(
        &self,
        tx_id: &str,
        workspace: &str,
        node_path: &str,
        property_path: &str,
        value: Value,
    ) -> Result<()> {
        let callback = self.callbacks.tx_update_property.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "Transaction update property callback not configured".to_string(),
            )
        })?;
        callback(
            tx_id.to_string(),
            workspace.to_string(),
            node_path.to_string(),
            property_path.to_string(),
            value,
        )
        .await
    }
}
