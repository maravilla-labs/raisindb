// SPDX-License-Identifier: BSL-1.1

//! Translation resolution helpers for repository handlers.
//!
//! Provides functions to resolve nodes with translations when a locale
//! parameter is specified in the request query.

use raisin_core::TranslationResolver;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::translations::LocaleCode;
use raisin_storage::{RepositoryManagementRepository, Storage};
use std::sync::Arc;

use crate::{error::ApiError, state::AppState};

/// Helper function to resolve a node with translations if a locale is specified.
///
/// If `lang` query parameter is present, this function:
/// 1. Gets the repository config to determine fallback chains
/// 2. Creates a TranslationResolver
/// 3. Applies translations using the fallback chain
/// 4. Returns None if the node is hidden in the requested locale
///
/// This keeps the main translation resolution logic in raisin-core for reusability
/// across different transport handlers.
pub(crate) async fn resolve_node_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    node: models::nodes::Node,
    lang: Option<String>,
    revision: &HLC,
) -> Result<Option<models::nodes::Node>, ApiError> {
    // If no lang parameter, return the node as-is
    let Some(lang_str) = lang else {
        return Ok(Some(node));
    };

    // Parse locale code
    let locale = LocaleCode::parse(&lang_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get repository config for fallback chains
    let repo_info = state
        .storage()
        .repository_management()
        .get_repository(tenant_id, repo)
        .await?
        .ok_or_else(|| ApiError::not_found("Repository not found"))?;

    // Create translation resolver
    let translation_repo = state.storage().translations().clone();
    let resolver =
        TranslationResolver::new(std::sync::Arc::new(translation_repo), repo_info.config);

    // Resolve node with translations
    let resolved_node = resolver
        .resolve_node(tenant_id, repo, branch, ws, node, &locale, revision)
        .await?;

    Ok(resolved_node)
}

/// Helper function to resolve multiple nodes with translations if a locale is specified.
///
/// Applies translation resolution to a vector of nodes, filtering out any that are
/// hidden in the requested locale.
///
/// # Performance
///
/// Uses `resolve_nodes_batch()` for 10-100x faster translation resolution compared
/// to individual `resolve_node()` calls. This method batch-fetches all translations
/// in a single RocksDB operation.
pub(crate) async fn resolve_nodes_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    nodes: Vec<models::nodes::Node>,
    lang: Option<String>,
    revision: &HLC,
) -> Result<Vec<models::nodes::Node>, ApiError> {
    // If no lang parameter, return nodes as-is
    let Some(lang_str) = lang else {
        return Ok(nodes);
    };

    // Parse locale code
    let locale = LocaleCode::parse(&lang_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get repository config for fallback chains
    let repo_info = state
        .storage()
        .repository_management()
        .get_repository(tenant_id, repo)
        .await?
        .ok_or_else(|| ApiError::not_found("Repository not found"))?;

    // Create translation resolver
    let translation_repo = state.storage().translations().clone();
    let resolver =
        TranslationResolver::new(std::sync::Arc::new(translation_repo), repo_info.config);

    // Batch resolve all nodes with translations (hidden nodes filtered out automatically)
    let resolved_nodes = resolver
        .resolve_nodes_batch(tenant_id, repo, branch, ws, nodes, &locale, revision)
        .await?;

    Ok(resolved_nodes)
}

/// Helper function to resolve NodeWithChildren recursively with translations.
///
/// Translates the node and recursively translates all child nodes.
pub(crate) async fn resolve_node_with_children_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    node_with_children: models::nodes::NodeWithChildren,
    resolver: &TranslationResolver<impl raisin_storage::TranslationRepository>,
    locale: &LocaleCode,
    revision: &HLC,
) -> Result<Option<models::nodes::NodeWithChildren>, ApiError> {
    use models::nodes::{ChildrenField, NodeWithChildren};

    // Translate the base node
    let translated_node = match resolver
        .resolve_node(
            tenant_id,
            repo,
            branch,
            ws,
            node_with_children.node,
            locale,
            revision,
        )
        .await?
    {
        Some(n) => n,
        None => return Ok(None), // Node is hidden
    };

    // Handle children based on type
    let translated_children = match node_with_children.children {
        ChildrenField::Names(names) => ChildrenField::Names(names),
        ChildrenField::Nodes(children) => {
            let mut translated = Vec::new();
            for child_box in children {
                let child = *child_box;
                if let Some(translated_child) = Box::pin(resolve_node_with_children_with_locale(
                    state, tenant_id, repo, branch, ws, child, resolver, locale, revision,
                ))
                .await?
                {
                    translated.push(Box::new(translated_child));
                }
            }
            ChildrenField::Nodes(translated)
        }
    };

    Ok(Some(NodeWithChildren {
        node: translated_node,
        children: translated_children,
    }))
}

/// Helper function to resolve Vec<NodeWithChildren> with translations.
pub(crate) async fn resolve_array_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    nodes: Vec<models::nodes::NodeWithChildren>,
    lang: Option<String>,
    revision: &HLC,
) -> Result<Vec<models::nodes::NodeWithChildren>, ApiError> {
    // If no lang parameter, return nodes as-is
    let Some(lang_str) = lang else {
        return Ok(nodes);
    };

    // Parse locale code
    let locale = LocaleCode::parse(&lang_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get repository config for fallback chains
    let repo_info = state
        .storage()
        .repository_management()
        .get_repository(tenant_id, repo)
        .await?
        .ok_or_else(|| ApiError::not_found("Repository not found"))?;

    // Create translation resolver
    let translation_repo = state.storage().translations().clone();
    let resolver = TranslationResolver::new(Arc::new(translation_repo), repo_info.config);

    // Resolve each node with children, filtering out hidden nodes
    let mut resolved_nodes = Vec::new();
    for node_with_children in nodes {
        if let Some(resolved) = resolve_node_with_children_with_locale(
            state,
            tenant_id,
            repo,
            branch,
            ws,
            node_with_children,
            &resolver,
            &locale,
            revision,
        )
        .await?
        {
            resolved_nodes.push(resolved);
        }
    }

    Ok(resolved_nodes)
}

/// Helper function to resolve HashMap<String, Node> (flat format) with translations.
///
/// # Performance
///
/// Uses batch translation resolution for all nodes at once instead of individual calls.
pub(crate) async fn resolve_flat_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    nodes: Vec<models::nodes::Node>,
    lang: Option<String>,
    revision: &HLC,
) -> Result<Vec<models::nodes::Node>, ApiError> {
    // If no lang parameter, return nodes as-is
    let Some(lang_str) = lang else {
        return Ok(nodes);
    };

    // Parse locale code
    let locale = LocaleCode::parse(&lang_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get repository config for fallback chains
    let repo_info = state
        .storage()
        .repository_management()
        .get_repository(tenant_id, repo)
        .await?
        .ok_or_else(|| ApiError::not_found("Repository not found"))?;

    // Create translation resolver
    let translation_repo = state.storage().translations().clone();
    let resolver = TranslationResolver::new(Arc::new(translation_repo), repo_info.config);

    // Batch resolve all nodes
    let resolved_vec = resolver
        .resolve_nodes_batch(tenant_id, repo, branch, ws, nodes, &locale, revision)
        .await?;

    Ok(resolved_vec)
}

/// Helper function to resolve DeepNode recursively with translations.
pub(crate) async fn resolve_deep_node_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    deep_node: models::nodes::DeepNode,
    resolver: &TranslationResolver<impl raisin_storage::TranslationRepository>,
    locale: &LocaleCode,
    revision: &HLC,
) -> Result<Option<models::nodes::DeepNode>, ApiError> {
    // Translate the base node
    let translated_node = match resolver
        .resolve_node(
            tenant_id,
            repo,
            branch,
            ws,
            deep_node.node,
            locale,
            revision,
        )
        .await?
    {
        Some(n) => n,
        None => return Ok(None), // Node is hidden
    };

    // Recursively translate children
    let mut translated_children = std::collections::HashMap::new();
    for (key, child) in deep_node.children {
        if let Some(translated_child) = Box::pin(resolve_deep_node_with_locale(
            state, tenant_id, repo, branch, ws, child, resolver, locale, revision,
        ))
        .await?
        {
            translated_children.insert(key, translated_child);
        }
    }

    Ok(Some(models::nodes::DeepNode {
        node: translated_node,
        children: translated_children,
    }))
}

/// Helper function to resolve HashMap<String, DeepNode> (nested format) with translations.
pub(crate) async fn resolve_nested_with_locale(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    ws: &str,
    nodes: std::collections::HashMap<String, models::nodes::DeepNode>,
    lang: Option<String>,
    revision: &HLC,
) -> Result<std::collections::HashMap<String, models::nodes::DeepNode>, ApiError> {
    // If no lang parameter, return nodes as-is
    let Some(lang_str) = lang else {
        return Ok(nodes);
    };

    // Parse locale code
    let locale = LocaleCode::parse(&lang_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get repository config for fallback chains
    let repo_info = state
        .storage()
        .repository_management()
        .get_repository(tenant_id, repo)
        .await?
        .ok_or_else(|| ApiError::not_found("Repository not found"))?;

    // Create translation resolver
    let translation_repo = state.storage().translations().clone();
    let resolver = TranslationResolver::new(Arc::new(translation_repo), repo_info.config);

    // Resolve each deep node
    let mut resolved_nodes = std::collections::HashMap::new();
    for (key, deep_node) in nodes {
        if let Some(resolved) = resolve_deep_node_with_locale(
            state, tenant_id, repo, branch, ws, deep_node, &resolver, &locale, revision,
        )
        .await?
        {
            resolved_nodes.insert(key, resolved);
        }
    }

    Ok(resolved_nodes)
}
