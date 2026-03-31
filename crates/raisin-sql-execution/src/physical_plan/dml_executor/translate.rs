// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! TRANSLATE execution for managing node translations.
//!
//! Updates translations for nodes in a specific locale by resolving
//! the filter to find target nodes, building a LocaleOverlay, and
//! storing it via the transaction context.

use crate::physical_plan::executor::{ExecutionContext, Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{AnalyzedTranslateFilter, AnalyzedTranslationValue};
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute a physical TRANSLATE operation.
///
/// Updates translations for nodes in a specific locale by:
/// 1. Resolving the filter to find target node(s) (by path, id, or node_type)
/// 2. Building a LocaleOverlay from node and block translations
/// 3. Storing the overlay via the transaction context
/// 4. Returning affected_rows count
pub async fn execute_translate<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    locale: &'a str,
    node_translations: &'a std::collections::HashMap<String, AnalyzedTranslationValue>,
    block_translations: &'a std::collections::HashMap<
        String,
        std::collections::HashMap<String, AnalyzedTranslationValue>,
    >,
    filter: &'a Option<AnalyzedTranslateFilter>,
    workspace: &'a Option<String>,
    branch_override: &'a Option<String>,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, Error> {
    use raisin_models::translations::{JsonPointer, LocaleOverlay};

    let workspace_id = workspace
        .as_ref()
        .map(|w| w.as_str())
        .unwrap_or(&ctx.workspace);

    let branch = branch_override
        .as_ref()
        .map(|b| b.as_str())
        .unwrap_or(&ctx.branch);

    tracing::debug!(
        "TRANSLATE: locale='{}', workspace='{}', {} node props, {} blocks",
        locale,
        workspace_id,
        node_translations.len(),
        block_translations.len()
    );

    // Step 1: Find target node(s) based on filter
    let node_ids = resolve_translate_targets(filter, workspace_id, branch, ctx).await?;

    if node_ids.is_empty() {
        tracing::debug!("TRANSLATE: No nodes matched the filter");
        let mut result_row = Row::new();
        result_row.insert("affected_rows".to_string(), PropertyValue::Integer(0));
        return Ok(Box::pin(stream::once(async move { Ok(result_row) })));
    }

    // Step 2: Build LocaleOverlay from translations
    let mut overlay_data = std::collections::HashMap::new();

    for (json_pointer, value) in node_translations {
        let prop_value = translation_value_to_property_value(value);
        overlay_data.insert(JsonPointer::new(json_pointer), prop_value);
    }

    for (block_uuid, block_props) in block_translations {
        for (json_pointer, value) in block_props {
            let prop_value = translation_value_to_property_value(value);
            let full_path = format!("/blocks/{}{}", block_uuid, json_pointer);
            overlay_data.insert(JsonPointer::new(&full_path), prop_value);
        }
    }

    let overlay = LocaleOverlay::properties(overlay_data);

    tracing::debug!(
        "TRANSLATE: Built overlay with {} fields for {} node(s)",
        overlay.len(),
        node_ids.len()
    );

    // Step 3: Store translation for each node
    let use_active_txn = {
        let tx_lock = ctx.transaction_context.read().await;
        tx_lock.is_some()
    };

    let affected_count = node_ids.len();

    if use_active_txn {
        tracing::debug!("TRANSLATE using active transaction context");
        let tx_lock = ctx.transaction_context.read().await;
        let txn_ctx = tx_lock.as_ref().ok_or_else(|| {
            Error::InvalidState("Transaction context lost during execution".to_string())
        })?;

        use raisin_storage::transactional::TransactionalContext;
        txn_ctx.set_branch(branch)?;

        for node_id in &node_ids {
            tracing::debug!("TRANSLATE storing translation for node {}", node_id);
            txn_ctx
                .store_translation(workspace_id, node_id, locale, overlay.clone())
                .await?;
        }
        drop(tx_lock);
    } else {
        tracing::debug!("TRANSLATE using auto-commit mode");
        use raisin_storage::transactional::TransactionalContext;
        let txn_ctx = ctx.storage.begin_context().await?;

        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(branch)?;

        let props: Vec<&str> = node_translations
            .keys()
            .chain(block_translations.keys())
            .map(|s| s.as_str())
            .collect();
        let props_str = props.join(",");
        let target = match filter {
            Some(AnalyzedTranslateFilter::Path(p)) => p.as_str(),
            Some(AnalyzedTranslateFilter::PathAndType { path, .. }) => path.as_str(),
            Some(AnalyzedTranslateFilter::Id(id)) => id.as_str(),
            Some(AnalyzedTranslateFilter::IdAndType { id, .. }) => id.as_str(),
            Some(AnalyzedTranslateFilter::NodeType(nt)) => nt.as_str(),
            None => "unknown",
        };
        let message = format!(
            "SQL TRANSLATE {} {} to locale '{}'",
            target, props_str, locale
        );
        txn_ctx.set_message(&message)?;
        txn_ctx.set_actor("sql-translate")?;

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        for node_id in &node_ids {
            tracing::debug!("TRANSLATE storing translation for node {}", node_id);
            txn_ctx
                .store_translation(workspace_id, node_id, locale, overlay.clone())
                .await?;
        }

        txn_ctx.commit().await?;
    }

    tracing::info!(
        "TRANSLATE: Updated {} node(s) for locale '{}' in workspace '{}'",
        affected_count,
        locale,
        workspace_id
    );

    let mut result_row = Row::new();
    result_row.insert(
        "affected_rows".to_string(),
        PropertyValue::Integer(affected_count as i64),
    );

    Ok(Box::pin(stream::once(async move { Ok(result_row) })))
}

/// Resolve translate filter to a list of target node IDs.
async fn resolve_translate_targets<S>(
    filter: &Option<AnalyzedTranslateFilter>,
    workspace_id: &str,
    branch: &str,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<String>, Error>
where
    S: Storage + 'static,
{
    match filter {
        Some(AnalyzedTranslateFilter::Path(path)) => {
            let node = ctx
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
                    path,
                    None,
                )
                .await?
                .ok_or_else(|| Error::NotFound(format!("Node at path '{}' not found", path)))?;

            check_translate_permission(&node, ctx, workspace_id, branch)?;
            Ok(vec![node.id])
        }
        Some(AnalyzedTranslateFilter::Id(id)) => {
            let node = ctx
                .storage
                .nodes()
                .get(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
                    id,
                    None,
                )
                .await?
                .ok_or_else(|| Error::NotFound(format!("Node with id '{}' not found", id)))?;

            check_translate_permission(&node, ctx, workspace_id, branch)?;
            Ok(vec![id.clone()])
        }
        Some(AnalyzedTranslateFilter::PathAndType { path, node_type }) => {
            let node = ctx
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
                    path,
                    None,
                )
                .await?
                .ok_or_else(|| Error::NotFound(format!("Node at path '{}' not found", path)))?;

            if node.node_type != *node_type {
                return Err(Error::Validation(format!(
                    "Node at path '{}' has type '{}', expected '{}'",
                    path, node.node_type, node_type
                )));
            }

            check_translate_permission(&node, ctx, workspace_id, branch)?;
            Ok(vec![node.id])
        }
        Some(AnalyzedTranslateFilter::IdAndType { id, node_type }) => {
            let node = ctx
                .storage
                .nodes()
                .get(
                    StorageScope::new(&ctx.tenant_id, &ctx.repo_id, branch, workspace_id),
                    id,
                    None,
                )
                .await?
                .ok_or_else(|| Error::NotFound(format!("Node with id '{}' not found", id)))?;

            if node.node_type != *node_type {
                return Err(Error::Validation(format!(
                    "Node with id '{}' has type '{}', expected '{}'",
                    id, node.node_type, node_type
                )));
            }

            check_translate_permission(&node, ctx, workspace_id, branch)?;
            Ok(vec![id.clone()])
        }
        Some(AnalyzedTranslateFilter::NodeType(_node_type)) => Err(Error::Validation(
            "TRANSLATE with WHERE node_type = '...' (bulk update) is not yet supported. \
                 Please use WHERE path = '...' or WHERE id = '...' to update individual nodes."
                .to_string(),
        )),
        None => Err(Error::Validation(
            "TRANSLATE requires a WHERE clause to identify target node(s)".to_string(),
        )),
    }
}

/// Check TRANSLATE permission on a node via RLS.
fn check_translate_permission<S: Storage>(
    node: &raisin_models::nodes::Node,
    ctx: &ExecutionContext<S>,
    workspace_id: &str,
    branch: &str,
) -> Result<(), Error> {
    if let Some(ref auth) = ctx.auth_context {
        use raisin_core::services::rls_filter;
        use raisin_models::permissions::{Operation, PermissionScope};
        let scope = PermissionScope::new(workspace_id, branch);
        if !rls_filter::can_perform(node, Operation::Translate, auth, &scope) {
            return Err(Error::PermissionDenied(format!(
                "Cannot translate node '{}'",
                node.id
            )));
        }
    }
    Ok(())
}

/// Convert AnalyzedTranslationValue to PropertyValue.
fn translation_value_to_property_value(value: &AnalyzedTranslationValue) -> PropertyValue {
    match value {
        AnalyzedTranslationValue::String(s) => PropertyValue::String(s.clone()),
        AnalyzedTranslationValue::Integer(i) => PropertyValue::Integer(*i),
        AnalyzedTranslationValue::Float(f) => PropertyValue::Float(*f),
        AnalyzedTranslationValue::Boolean(b) => PropertyValue::Boolean(*b),
        AnalyzedTranslationValue::Null => PropertyValue::Null,
    }
}
