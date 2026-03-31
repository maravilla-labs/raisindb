// SPDX-License-Identifier: BSL-1.1
//! Command execution handlers for RaisinDB repository operations.
//!
//! This module contains the `repo_execute_command` function which handles
//! all SQL-style commands like rename, move, copy, publish, translate, etc.

mod common;
mod node_ops;
mod publishing;
mod relations;
mod transactions;
mod translations;
mod versioning;

use axum::http::StatusCode;
use axum::Json;
use raisin_models::auth::AuthContext;
use raisin_storage::{BranchRepository, Storage};

use crate::error::ApiError;
use crate::state::AppState;
use crate::types::CommandBody;

use common::CommandContext;

/// Execute a command on a node in the repository.
///
/// This function handles all command types including:
/// - Node operations: rename, move, copy, copy_tree, delete
/// - Publishing: publish, unpublish, publish_tree, unpublish_tree
/// - Ordering: reorder
/// - Versioning: create_version, restore_version, delete_version, update_version_note
/// - Audit: audit_log
/// - Transactions: commit, save, create
/// - Relations: add-relation, remove-relation
/// - Translations: translate, delete-translation, hide-in-locale, unhide-in-locale
pub async fn repo_execute_command(
    state: &AppState,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    command: &str,
    params: CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    tracing::info!(
        "COMMAND: {}, tenant={}, repo={}, branch={}, ws={}, path={}",
        command,
        tenant_id,
        repository,
        branch,
        ws,
        path
    );

    // Get branch HEAD revision and bound queries to it for snapshot isolation
    let mut nodes_svc = state.node_service_for_context(tenant_id, repository, branch, ws, auth.clone());
    let branch_head = state
        .storage()
        .branches()
        .get_branch(tenant_id, repository, branch)
        .await?
        .map(|info| info.head);
    if let Some(head) = branch_head {
        nodes_svc = nodes_svc.at_revision(head);
    }

    // Create command context
    let mut ctx = CommandContext {
        state,
        tenant_id,
        repository,
        branch,
        ws,
        path,
        params,
        auth,
        nodes_svc,
        branch_head,
    };

    match command {
        // Node operations
        "rename" => node_ops::handle_rename(&mut ctx).await,
        "move" => node_ops::handle_move(&mut ctx).await,
        "copy" => node_ops::handle_copy(&mut ctx).await,
        "copy_tree" => node_ops::handle_copy_tree(&mut ctx).await,
        "reorder" => node_ops::handle_reorder(&mut ctx).await,

        // Publishing
        "publish" => publishing::handle_publish(&mut ctx).await,
        "publish_tree" => publishing::handle_publish_tree(&mut ctx).await,
        "unpublish" => publishing::handle_unpublish(&mut ctx).await,
        "unpublish_tree" => publishing::handle_unpublish_tree(&mut ctx).await,

        // Versioning and audit
        "create_version" => versioning::handle_create_version(&mut ctx).await,
        "restore_version" => versioning::handle_restore_version(&mut ctx).await,
        "delete_version" => versioning::handle_delete_version(&mut ctx).await,
        "update_version_note" => versioning::handle_update_version_note(&mut ctx).await,
        "audit_log" => versioning::handle_audit_log(&mut ctx).await,

        // Transactions
        "commit" => transactions::handle_commit(&mut ctx).await,
        "save" => transactions::handle_save(&mut ctx).await,
        "create" => transactions::handle_create(&mut ctx).await,
        "delete" => transactions::handle_delete(&mut ctx).await,

        // Relations
        "add-relation" => relations::handle_add_relation(&mut ctx).await,
        "remove-relation" => relations::handle_remove_relation(&mut ctx).await,

        // Translations
        "translate" => translations::handle_translate(&mut ctx).await,
        "delete-translation" => translations::handle_delete_translation(&mut ctx).await,
        "hide-in-locale" => translations::handle_hide_in_locale(&mut ctx).await,
        "unhide-in-locale" => translations::handle_unhide_in_locale(&mut ctx).await,
        "translation-staleness" => translations::handle_translation_staleness(&mut ctx).await,
        "acknowledge-staleness" => translations::handle_acknowledge_staleness(&mut ctx).await,

        _ => Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "COMMAND_NOT_IMPLEMENTED",
            format!("Unknown command: {}", command),
        )),
    }
}
