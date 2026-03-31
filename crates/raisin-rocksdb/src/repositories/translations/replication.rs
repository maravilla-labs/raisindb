//! Operation capture helpers for replication.

use raisin_models::translations::LocaleOverlay;
use std::sync::Arc;

/// Capture a translation operation for replication
///
/// Determines whether to capture SetTranslation or DeleteTranslation based on the overlay type.
pub(super) async fn capture_translation_operation(
    operation_capture: &Arc<crate::OperationCapture>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    node_id: String,
    locale: String,
    property_name: String,
    overlay: &LocaleOverlay,
    actor: String,
) {
    if !operation_capture.is_enabled() {
        return;
    }

    match overlay {
        LocaleOverlay::Hidden => {
            // Hidden overlay means deletion
            let _ = operation_capture
                .capture_delete_translation(
                    tenant_id,
                    repo_id,
                    branch,
                    node_id,
                    locale,
                    property_name,
                    actor,
                )
                .await;
        }
        LocaleOverlay::Properties { data } => {
            // Properties overlay means setting translation
            let value = serde_json::to_value(data).unwrap_or(serde_json::json!({}));
            let _ = operation_capture
                .capture_set_translation(
                    tenant_id,
                    repo_id,
                    branch,
                    node_id,
                    locale,
                    property_name,
                    value,
                    actor,
                )
                .await;
        }
    }
}

/// Capture a node-level translation operation
pub(super) async fn capture_node_translation(
    operation_capture: Option<&Arc<crate::OperationCapture>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    locale: &str,
    overlay: &LocaleOverlay,
    actor: &str,
) {
    if let Some(capture) = operation_capture {
        capture_translation_operation(
            capture,
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            node_id.to_string(),
            locale.to_string(),
            "properties".to_string(), // Generic property name for node-level translations
            overlay,
            actor.to_string(),
        )
        .await;
    }
}

/// Capture a block-level translation operation
pub(super) async fn capture_block_translation(
    operation_capture: Option<&Arc<crate::OperationCapture>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_id: &str,
    locale: &str,
    block_uuid: &str,
    overlay: &LocaleOverlay,
    actor: &str,
) {
    if let Some(capture) = operation_capture {
        capture_translation_operation(
            capture,
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            node_id.to_string(),
            locale.to_string(),
            format!("block::{}", block_uuid),
            overlay,
            actor.to_string(),
        )
        .await;
    }
}
