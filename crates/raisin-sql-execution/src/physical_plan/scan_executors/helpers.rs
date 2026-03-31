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

use raisin_core::services::translation_resolver::TranslationResolver;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::LocaleCode;
use raisin_sql::analyzer::{BinaryOperator, Expr, Literal, TypedExpr};
use raisin_storage::Storage;
use std::sync::Arc;

use crate::physical_plan::executor::ExecutionContext;

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
