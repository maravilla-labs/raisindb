// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `LocalizedPathLookup` (plan Phase 12): `WHERE locale = 'fr' AND
//! __localized_path = '/produits/chaise'` through the storage's localized
//! name lookup — the same core lookup the HTTP and WS surfaces call.
//!
//! For each locale of the statement it resolves the path, and emits the node
//! only when its canonical localized path IS the requested one: the operator
//! is exactly the predicate it replaces (a canonical name used where the node
//! has a translated name, which the HTTP lookup answers with a redirect hint, is not
//! `__localized_path = …`). Missing, hidden and forbidden emit nothing.

use super::helpers::{get_locales_to_use, resolve_node_for_locale, rls_filter_node_graph};
use super::node_to_row::node_to_row_owned;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use async_stream::try_stream;
use raisin_error::Error;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

pub async fn execute_localized_path_lookup<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let PhysicalPlan::LocalizedPathLookup {
        tenant_id,
        repo_id,
        branch,
        workspace,
        table,
        alias,
        path,
        projection,
    } = plan.clone()
    else {
        return Err(Error::Validation(
            "Invalid plan for localized path lookup".to_string(),
        ));
    };
    let source = ctx.storage.localized_names().ok_or_else(|| {
        Error::Validation("localized path lookup is not supported by this backend".to_string())
    })?;
    let storage = ctx.storage.clone();
    let ctx = ctx.clone();

    Ok(Box::pin(try_stream! {
        let qualifier = alias.clone().unwrap_or_else(|| table.clone());
        let snapshot = ctx.statement_snapshot().await?;
        let scope = StorageScope::new(&tenant_id, &repo_id, &branch, &workspace);
        for locale in get_locales_to_use(&ctx) {
            let Some(found) = source.resolve(scope, &locale, &path, Some(&snapshot))? else {
                continue;
            };
            if found.canonical_localized_path != path {
                continue;
            }
            let Some(node) = super::helpers::row_node(&*storage, scope, raisin_storage::NodeLocator::Id(found.node_id.clone()), Some(&snapshot)).await? else {
                continue;
            };
            let node = match &ctx.auth_context {
                Some(auth) => {
                    let permission_scope = PermissionScope::new(&workspace, &branch);
                    match rls_filter_node_graph(&*storage, node, auth, &permission_scope, &tenant_id, &repo_id, &branch, Some(&snapshot)).await {
                        Some(n) => n,
                        None => continue,
                    }
                }
                None => node,
            };
            let Some(translated) = resolve_node_for_locale(node, &ctx, &locale).await? else {
                continue;
            };
            // The operator emits only when the canonical localized path IS the
            // requested one, so the row's `__localized_path` is known.
            let known = super::node_to_row::OrderContext {
                localized_path: Some(&found.canonical_localized_path),
                ..Default::default()
            };
            yield node_to_row_owned(translated, &qualifier, &workspace, &projection, &ctx, &locale, Some(&known)).await?;
        }
    }))
}
