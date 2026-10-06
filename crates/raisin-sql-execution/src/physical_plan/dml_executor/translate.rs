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
use raisin_core::{NodeRef, NodeService, TranslationWriteTarget};
use raisin_error::Error;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{AnalyzedTranslateFilter, AnalyzedTranslationValue};
use raisin_storage::Storage;

/// Merge this statement's pointers onto whatever the node already has for the
/// locale.
///
/// A store is a whole-overlay put, so building the overlay from just the SET list
/// made every `UPDATE … FOR LOCALE` a silent replace of that locale: translating a
/// page one block per statement, or running a machine pass after a human one, kept
/// only the last write and let everything else fall back to the base language with
/// nothing reported. `get_translation` here is read-your-writes, so several
/// statements inside one transaction accumulate correctly.
///
/// A `Hidden` tombstone is not a partial overlay: writing translations for a node
/// hidden in this locale replaces the tombstone, which is how it was un-hidden
/// before and stays so.
async fn merged_overlay(
    txn_ctx: &dyn raisin_storage::transactional::TransactionalContext,
    workspace_id: &str,
    node_id: &str,
    locale: &str,
    incoming: &std::collections::HashMap<raisin_models::translations::JsonPointer, PropertyValue>,
) -> Result<raisin_models::translations::LocaleOverlay, Error> {
    use raisin_models::translations::LocaleOverlay;

    let mut data = match txn_ctx.get_translation(workspace_id, node_id, locale).await {
        Ok(Some(LocaleOverlay::Properties { data })) => data,
        _ => std::collections::HashMap::new(),
    };
    for (pointer, value) in incoming {
        // NULL CLEARS one pointer. Merging alone would make a translation
        // permanent: once a field had a locale value there was no way to take it
        // back short of dropping the whole locale, so "this shouldn't be
        // translated after all" was not expressible. The field then falls back to
        // the base language, which is what removing a translation means.
        if matches!(value, PropertyValue::Null) {
            data.remove(pointer);
        } else {
            data.insert(pointer.clone(), value.clone());
        }
    }
    Ok(LocaleOverlay::Properties { data })
}

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
        "TRANSLATE: locale='{}', workspace='{}', {} translations",
        locale,
        workspace_id,
        node_translations.len()
    );

    // Step 1: Find target node(s) based on filter
    let targets = resolve_translate_targets(filter, workspace_id, branch, ctx).await?;

    if targets.is_empty() {
        tracing::debug!("TRANSLATE: No nodes matched the filter");
        let mut result_row = Row::new();
        result_row.insert("affected_rows".to_string(), PropertyValue::Integer(0));
        return Ok(Box::pin(stream::once(async move { Ok(result_row) })));
    }

    // Step 2: Build LocaleOverlay from translations.
    //
    // All translations are flat JsonPointers — plain (`/title`) or uuid-indexed
    // (`/sections/s1/features/f1/title`). The resolver navigates arrays by
    // matching uuid segments against item `uuid`s, so deep nesting works without
    // any special-casing here.
    let mut overlay_data = std::collections::HashMap::new();

    for (json_pointer, value) in node_translations {
        let prop_value = translation_value_to_property_value(value);
        overlay_data.insert(JsonPointer::new(json_pointer), prop_value);
    }

    tracing::debug!(
        "TRANSLATE: Built overlay with {} fields for {} node(s)",
        overlay_data.len(),
        targets.len()
    );

    // Step 3: Store translation for each node
    let use_active_txn = {
        let tx_lock = ctx.transaction_context.read().await;
        tx_lock.is_some()
    };

    let affected_count = targets.len();

    if use_active_txn {
        tracing::debug!("TRANSLATE using active transaction context");
        let tx_lock = ctx.transaction_context.read().await;
        let txn_ctx = tx_lock.as_ref().ok_or_else(|| {
            Error::InvalidState("Transaction context lost during execution".to_string())
        })?;

        use raisin_storage::transactional::TransactionalContext;
        txn_ctx.set_branch(branch)?;

        for target in &targets {
            let node_id = target.node.id.as_str();
            tracing::debug!("TRANSLATE storing translation for node {}", node_id);
            let overlay = merged_overlay(
                txn_ctx.as_ref(),
                workspace_id,
                node_id,
                locale,
                &overlay_data,
            )
            .await?;
            txn_ctx
                .store_translation(workspace_id, node_id, locale, overlay)
                .await?;
        }
        drop(tx_lock);
    } else {
        tracing::debug!("TRANSLATE using auto-commit mode");
        use raisin_storage::transactional::TransactionalContext;
        let txn_ctx = ctx.storage.begin_context().await?;

        txn_ctx.set_tenant_repo(&ctx.tenant_id, &ctx.repo_id)?;
        txn_ctx.set_branch(branch)?;

        let props: Vec<&str> = node_translations.keys().map(|s| s.as_str()).collect();
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
        // The shared policy named the actor; every target was authorized
        // for the same caller, so they agree.
        if let Some(target) = targets.first() {
            txn_ctx.set_actor(&target.actor)?;
        }

        let auth = ctx
            .auth_context
            .clone()
            .unwrap_or_else(AuthContext::anonymous);
        txn_ctx.set_auth_context(auth)?;

        for target in &targets {
            let node_id = target.node.id.as_str();
            tracing::debug!("TRANSLATE storing translation for node {}", node_id);
            let overlay = merged_overlay(
                txn_ctx.as_ref(),
                workspace_id,
                node_id,
                locale,
                &overlay_data,
            )
            .await?;
            txn_ctx
                .store_translation(workspace_id, node_id, locale, overlay)
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

/// Resolve the translate filter to the node(s) the caller may translate.
///
/// Everything that is POLICY — finding the node, treating an unreadable one as
/// not found, requiring `Translate` on it (graph `RELATES` conditions
/// included), keying by id, naming the actor — is
/// [`NodeService::authorize_translation_write`], the same call the HTTP and
/// WebSocket translation commands make, so the three surfaces cannot drift
/// apart again. Only the `node_type` guard is SQL's own.
///
/// SQL runs with no auth context as the system (no RLS), as every scan here
/// does; the shared check denies on no auth, so that is said explicitly.
async fn resolve_translate_targets<S>(
    filter: &Option<AnalyzedTranslateFilter>,
    workspace_id: &str,
    branch: &str,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<TranslationWriteTarget>, Error>
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
{
    let (node_ref, expected_type) = match filter {
        Some(AnalyzedTranslateFilter::Path(path)) => (NodeRef::Path(path), None),
        Some(AnalyzedTranslateFilter::Id(id)) => (NodeRef::Id(id), None),
        Some(AnalyzedTranslateFilter::PathAndType { path, node_type }) => {
            (NodeRef::Path(path), Some(node_type))
        }
        Some(AnalyzedTranslateFilter::IdAndType { id, node_type }) => {
            (NodeRef::Id(id), Some(node_type))
        }
        Some(AnalyzedTranslateFilter::NodeType(_node_type)) => {
            return Err(Error::Validation(
                "TRANSLATE with WHERE node_type = '...' (bulk update) is not yet supported. \
                 Please use WHERE path = '...' or WHERE id = '...' to update individual nodes."
                    .to_string(),
            ))
        }
        None => {
            return Err(Error::Validation(
                "TRANSLATE requires a WHERE clause to identify target node(s)".to_string(),
            ))
        }
    };

    let auth = ctx.auth_context.clone().unwrap_or_else(AuthContext::system);
    let nodes = NodeService::new_with_context(
        ctx.storage.clone(),
        ctx.tenant_id.to_string(),
        ctx.repo_id.to_string(),
        branch.to_string(),
        workspace_id.to_string(),
    )
    .with_auth(auth);
    let target = nodes.authorize_translation_write(node_ref).await?;

    if let Some(node_type) = expected_type {
        if target.node.node_type != *node_type {
            return Err(Error::Validation(format!(
                "Node '{}' has type '{}', expected '{}'",
                target.node.path, target.node.node_type, node_type
            )));
        }
    }
    Ok(vec![target])
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
