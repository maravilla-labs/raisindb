// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Shared helper functions for scan executors.
//!
//! Provides utility functions used across multiple scan implementations:
//! - Property predicate extraction from filter expressions
//! - Locale resolution for translation queries
//! - Node translation resolution

use raisin_core::services::rls_filter;
use raisin_core::services::translation_resolver::TranslationResolver;
use raisin_error::Error;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_models::translations::LocaleCode;
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};
use raisin_storage::{scope::BranchScope, Storage};
use std::sync::Arc;

use crate::physical_plan::executor::ExecutionContext;

/// Apply RLS to a single node, building a cache-backed graph resolver from
/// `storage` so `RELATES … VIA` conditions are evaluated against the relation
/// graph. This is the async counterpart to `rls_filter::filter_node` used
/// throughout the scan executors; without it, graph-relationship RLS conditions
/// fail closed. Relations are evaluated at `max_revision` (or latest when None).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn rls_filter_node_graph<S: Storage>(
    storage: &S,
    node: Node,
    auth: &AuthContext,
    scope: &PermissionScope,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    max_revision: Option<&HLC>,
) -> Option<Node> {
    // Hot-path fast lane: when no permission uses a graph (`RELATES`) condition,
    // skip resolver construction entirely and take the synchronous path.
    if !auth.uses_graph_rls() {
        return rls_filter::filter_node(node, auth, scope);
    }
    let rev = max_revision.copied().unwrap_or_else(HLC::now);
    let resolver = storage.graph_resolver(BranchScope::new(tenant_id, repo_id, branch), &rev);
    rls_filter::filter_node_async(node, auth, scope, resolver.as_deref()).await
}

/// Extract a property predicate from a filter expression for filter-first fallback.
///
/// This function recursively searches the filter expression tree for equality predicates
/// that can be used with the property index. It looks for patterns like:
/// - `node_type = 'SomeType'` -> returns ("__node_type", PropertyValue::String("SomeType"))
/// - `properties ->> 'key' = 'value'` -> returns ("key", PropertyValue::String("value"))
///
/// Returns the first suitable predicate found, prioritizing node_type for selectivity.
pub(super) fn extract_property_predicate_from_filter(
    filter: &TypedExpr,
) -> Option<(String, PropertyValue)> {
    match &filter.expr {
        // Handle AND expressions - check both sides
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            // Try left side first, prefer node_type predicates
            if let Some(pred) = extract_property_predicate_from_filter(left) {
                if pred.0 == "__node_type" {
                    return Some(pred);
                }
                // Keep looking for node_type on right side
                if let Some(right_pred) = extract_property_predicate_from_filter(right) {
                    if right_pred.0 == "__node_type" {
                        return Some(right_pred);
                    }
                }
                // No node_type found, return first property predicate
                return Some(pred);
            }
            extract_property_predicate_from_filter(right)
        }

        // Handle equality: column = value
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Check for node_type = 'value'
            if let Expr::Column { column, .. } = &left.expr {
                if column.to_lowercase() == "node_type" {
                    if let Expr::Literal(Literal::Text(value)) = &right.expr {
                        return Some((
                            "__node_type".to_string(),
                            PropertyValue::String(value.clone()),
                        ));
                    }
                }
            }

            // Check for properties ->> 'key' = 'value' (JsonExtractText pattern)
            if let Expr::JsonExtractText { object, key } = &left.expr {
                if let Expr::Column { column, .. } = &object.expr {
                    if column.to_lowercase() == "properties" {
                        if let Expr::Literal(Literal::Text(prop_key)) = &key.expr {
                            if let Expr::Literal(Literal::Text(prop_value)) = &right.expr {
                                return Some((
                                    prop_key.clone(),
                                    PropertyValue::String(prop_value.clone()),
                                ));
                            }
                        }
                    }
                }
            }

            None
        }

        // Handle OR expressions - we can't use these for index lookups safely
        Expr::BinaryOp {
            op: BinaryOperator::Or,
            ..
        } => None,

        // Other expression types don't have extractable property predicates
        _ => None,
    }
}

/// Determine which locales to use for a query.
///
/// Returns a vec of locale strings to process. If no locale is specified
/// in the query, uses the default language from repository configuration.
pub(super) fn get_locales_to_use<S: Storage>(ctx: &ExecutionContext<S>) -> Vec<String> {
    if ctx.locales.is_empty() {
        // No locale specified in query, use default from repository configuration
        vec![ctx.default_language.to_string()]
    } else {
        // Use locales from WHERE clause
        ctx.locales.to_vec()
    }
}

/// Resolve translation for a single node.
///
/// If repository_config is set and the locale differs from the default language,
/// this function applies translations using the TranslationResolver.
///
/// Returns `Some(translated_node)` if the node should be visible in this locale,
/// or `None` if the node is hidden in this locale.
pub(super) async fn resolve_node_for_locale<S: Storage>(
    node: Node,
    ctx: &ExecutionContext<S>,
    locale: &str,
) -> Result<Option<Node>, Error> {
    resolve_node_for_locale_as(node, ctx, locale, true).await
}

/// [`resolve_node_for_locale`] for a scan that may not read properties.
///
/// With `with_properties == false` the node was read without its property map
/// (`ListOptions::skip_properties`), so there is nothing to merge an overlay
/// into: only the visibility question is answered, which also skips reading
/// the block overlays.
pub(super) async fn resolve_node_for_locale_as<S: Storage>(
    node: Node,
    ctx: &ExecutionContext<S>,
    locale: &str,
    with_properties: bool,
) -> Result<Option<Node>, Error> {
    // Skip translation if:
    // 1. No repository_config is set (translation not configured)
    // 2. The locale matches the default language (no translation needed)
    let config = match &ctx.repository_config {
        Some(config) => config,
        None => return Ok(Some(node)), // No translation configured, return as-is
    };

    // If querying the default language, no translation needed
    if locale == ctx.default_language.as_ref() {
        return Ok(Some(node));
    }

    // Parse locale code
    let locale_code = LocaleCode::parse(locale)
        .map_err(|e| Error::Validation(format!("Invalid locale '{}': {}", locale, e)))?;

    // Get revision for translation lookup
    let revision = ctx.max_revision.unwrap_or_else(raisin_hlc::HLC::now);

    // Create the translation resolver
    let translation_repo = ctx.storage.translations();
    let resolver = TranslationResolver::new(Arc::new(translation_repo.clone()), config.clone());

    if !with_properties {
        let visible = resolver
            .is_visible(
                &ctx.tenant_id,
                &ctx.repo_id,
                &ctx.branch,
                &ctx.workspace,
                &node.id,
                &locale_code,
                &revision,
            )
            .await?;
        return Ok(visible.then_some(node));
    }

    // Resolve translation for this node
    resolver
        .resolve_node(
            &ctx.tenant_id,
            &ctx.repo_id,
            &ctx.branch,
            &ctx.workspace,
            node,
            &locale_code,
            &revision,
        )
        .await
}

/// Columns `node_to_row` fills from the node record itself, never from
/// `properties`. Any other projected name is looked up IN `properties`
/// (`insert_property_fields`), so it needs them decoded.
const NODE_RECORD_COLUMNS: &[&str] = &[
    "id",
    "path",
    "name",
    "node_type",
    "__node_type",
    "archetype",
    "created_at",
    "updated_at",
    "created_by",
    "updated_by",
    "published_at",
    "published_by",
    "version",
    "depth",
    "parent_name",
    "locale",
    "__workspace",
    "__order",
    "__tree_order",
    "embedding",
];

/// Does a scan with this projection need each node's `properties` decoded?
///
/// The projection is the set of columns the plan above the scan reads —
/// output columns AND the columns its residual filters evaluate — so when it
/// holds only node-record columns, the property map is decoded for nothing.
/// That decode is most of what reading a content node costs.
///
/// Also yes whenever row-level security may evaluate a CONDITION: conditions
/// read node properties (`rls_filter::context`), and an empty map would make
/// them decide differently. System callers, system admins and grants without
/// conditions never look.
pub(super) fn scan_needs_properties<S: Storage>(
    projection: &Option<Vec<String>>,
    ctx: &ExecutionContext<S>,
) -> bool {
    let Some(columns) = projection else {
        return true;
    };
    if columns
        .iter()
        .any(|c| !NODE_RECORD_COLUMNS.contains(&c.as_str()))
    {
        return true;
    }
    ctx.auth_context.as_ref().is_some_and(|auth| {
        !auth.is_system
            && auth.permissions().is_none_or(|p| {
                !p.is_system_admin && p.permissions.iter().any(|perm| perm.condition.is_some())
            })
    })
}

/// Would row-level security filter this caller's reads?
///
/// The COUNT fast paths (`CountScan` / `PropertyIndexCountScan`) count storage
/// keys directly, without ever materializing a node or passing it through
/// `rls_filter`. That is correct ONLY for a caller RLS would wave through
/// wholesale — a system caller or a system admin. For anyone else it leaks the
/// row count (and, with a property filter, an existence oracle) of workspaces
/// the caller cannot read a single row of. So the count executors must fall
/// back to an RLS-aware count whenever this returns true.
///
/// Mirrors the allow-all short-circuits in `rls_filter::filter_node`:
///   - no `auth_context`  -> internal caller, no RLS (same as every scan
///     executor, which only filters when `auth_context` is `Some`);
///   - `is_system`        -> bypasses RLS;
///   - `is_system_admin`  -> bypasses RLS;
///   - permissions unresolved (`None`) -> RLS denies every node, so the count
///     must be filtered (it will be 0), never answered from the raw key count.
pub(super) fn auth_requires_rls<S: Storage>(ctx: &ExecutionContext<S>) -> bool {
    ctx.auth_context.as_ref().is_some_and(|auth| {
        !auth.is_system && auth.permissions().is_none_or(|p| !p.is_system_admin)
    })
}
