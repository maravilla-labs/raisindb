// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! User node management for identity authentication.
//!
//! This module handles creating and looking up `raisin:User` nodes in the
//! repository's `raisin:access_control` workspace.

#[cfg(feature = "storage-rocksdb")]
use std::sync::Arc;

/// Convert email to a safe node name.
#[cfg(feature = "storage-rocksdb")]
pub fn email_to_node_name(email: &str) -> String {
    email
        .to_lowercase()
        .replace('@', "-at-")
        .replace('.', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

/// Create or get user node inline, returning the home path.
///
/// This creates the `raisin:User` node in the `raisin:access_control` workspace
/// synchronously during registration/login, so the home path is immediately
/// available in the JWT token.
#[cfg(feature = "storage-rocksdb")]
pub async fn ensure_user_node(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    identity_id: &str,
    email: &str,
    display_name: Option<&str>,
    default_roles: &[String],
) -> Result<String, String> {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::Node;
    use raisin_storage::{NodeRepository, RepositoryManagementRepository, Storage, StorageScope};
    use std::collections::HashMap;

    let workspace = "raisin:access_control";
    let users_path = "/users/internal";
    let node_name = email_to_node_name(email);
    let user_path = format!("{}/{}", users_path, node_name);

    // Get default branch from repository config
    let default_branch = match Storage::repository_management(storage.as_ref())
        .get_repository(tenant_id, repo_id)
        .await
    {
        Ok(Some(repo)) => repo.config.default_branch.clone(),
        _ => "main".to_string(),
    };

    // Use NodeService with system auth context (bypasses RLS)
    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        default_branch.clone(),
        workspace.to_string(),
    )
    .with_auth(AuthContext::system());

    // Check if user node already exists at expected path
    if let Some(existing_node) = node_service
        .get_by_path(&user_path)
        .await
        .map_err(|e| e.to_string())?
    {
        tracing::debug!(
            user_node_id = %existing_node.id,
            user_path = %existing_node.path,
            "User node already exists at expected path"
        );
        return Ok(existing_node.path);
    }

    // Path check failed - search by email property (node might be at different path)
    let email_value = PropertyValue::String(email.to_string());
    let nodes_by_email = Storage::nodes(storage.as_ref())
        .find_by_property(
            StorageScope::new(tenant_id, repo_id, &default_branch, workspace),
            "email",
            &email_value,
        )
        .await
        .map_err(|e| e.to_string())?;

    if let Some(existing_node) = nodes_by_email
        .into_iter()
        .find(|n| n.node_type == "raisin:User")
    {
        tracing::info!(
            user_node_id = %existing_node.id,
            user_path = %existing_node.path,
            "Found existing user node by email property"
        );
        return Ok(existing_node.path);
    }

    // Also search by user_id (identity_id) property
    let user_id_value = PropertyValue::String(identity_id.to_string());
    let nodes_by_user_id = Storage::nodes(storage.as_ref())
        .find_by_property(
            StorageScope::new(tenant_id, repo_id, &default_branch, workspace),
            "user_id",
            &user_id_value,
        )
        .await
        .map_err(|e| e.to_string())?;

    if let Some(existing_node) = nodes_by_user_id
        .into_iter()
        .find(|n| n.node_type == "raisin:User")
    {
        tracing::info!(
            user_node_id = %existing_node.id,
            user_path = %existing_node.path,
            "Found existing user node by user_id property"
        );
        return Ok(existing_node.path);
    }

    // No existing node found - create new one

    // Build role references
    let role_refs: Vec<PropertyValue> = default_roles
        .iter()
        .map(|role| PropertyValue::String(format!("/roles/{}", role)))
        .collect();

    // Build user properties
    let mut properties = HashMap::new();
    properties.insert(
        "user_id".to_string(),
        PropertyValue::String(identity_id.to_string()),
    );
    properties.insert(
        "email".to_string(),
        PropertyValue::String(email.to_string()),
    );
    properties.insert(
        "display_name".to_string(),
        PropertyValue::String(
            display_name
                .unwrap_or_else(|| email.split('@').next().unwrap_or("User"))
                .to_string(),
        ),
    );
    properties.insert(
        "status".to_string(),
        PropertyValue::String("active".to_string()),
    );
    properties.insert("roles".to_string(), PropertyValue::Array(role_refs));
    properties.insert(
        "created_at".to_string(),
        PropertyValue::String(chrono::Utc::now().to_rfc3339()),
    );

    // Create user node with initial_structure (profile, inbox, outbox children)
    let user_node = Node {
        id: String::new(),
        node_type: "raisin:User".to_string(),
        name: node_name,
        path: String::new(),
        workspace: Some(workspace.to_string()),
        properties,
        ..Default::default()
    };

    let created_node = node_service
        .add_deep_node(users_path, user_node)
        .await
        .map_err(|e| e.to_string())?;

    tracing::info!(
        user_node_id = %created_node.id,
        user_path = %created_node.path,
        "User node created successfully with initial_structure"
    );

    Ok(created_node.path)
}

/// Look up existing user node by email, returning the home path.
#[cfg(feature = "storage-rocksdb")]
pub async fn lookup_user_home(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    tenant_id: &str,
    repo_id: &str,
    email: &str,
) -> Result<Option<String>, String> {
    use raisin_core::services::node_service::NodeService;
    use raisin_models::auth::AuthContext;
    use raisin_storage::{RepositoryManagementRepository, Storage};

    let workspace = "raisin:access_control";
    let node_name = email_to_node_name(email);
    let user_path = format!("/users/internal/{}", node_name);

    // Get default branch from repository config
    let default_branch = match Storage::repository_management(storage.as_ref())
        .get_repository(tenant_id, repo_id)
        .await
    {
        Ok(Some(repo)) => repo.config.default_branch.clone(),
        _ => "main".to_string(),
    };

    let node_service: NodeService<raisin_rocksdb::RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant_id.to_string(),
        repo_id.to_string(),
        default_branch,
        workspace.to_string(),
    )
    .with_auth(AuthContext::system());

    match node_service.get_by_path(&user_path).await {
        Ok(Some(node)) => Ok(Some(node.path)),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
