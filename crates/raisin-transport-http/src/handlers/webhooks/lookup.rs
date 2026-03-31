// SPDX-License-Identifier: BSL-1.1

//! Trigger node lookup functions.

use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::error::ApiError;
use crate::state::AppState;

use super::helpers::property_as_string;
use super::types::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, TENANT_ID};

/// Find trigger node by webhook_id property
pub(super) async fn find_trigger_by_webhook_id(
    state: &AppState,
    repo: &str,
    webhook_id: &str,
) -> Result<Node, ApiError> {
    let triggers = state
        .storage
        .nodes()
        .list_by_type(
            StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE),
            "raisin:Trigger",
            raisin_storage::ListOptions::default(),
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    for trigger in triggers {
        if let Some(id) = property_as_string(trigger.properties.get("webhook_id")) {
            if id == webhook_id {
                return Ok(trigger);
            }
        }
    }

    Err(ApiError::not_found(format!(
        "Webhook not found: {}",
        webhook_id
    )))
}

/// Find trigger node by unique name property
pub(super) async fn find_trigger_by_name(
    state: &AppState,
    repo: &str,
    name: &str,
) -> Result<Node, ApiError> {
    let triggers = state
        .storage
        .nodes()
        .list_by_type(
            StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE),
            "raisin:Trigger",
            raisin_storage::ListOptions::default(),
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    for trigger in triggers {
        if let Some(trigger_name) = property_as_string(trigger.properties.get("name")) {
            if trigger_name == name {
                return Ok(trigger);
            }
        }
    }

    Err(ApiError::not_found(format!("Trigger not found: {}", name)))
}
