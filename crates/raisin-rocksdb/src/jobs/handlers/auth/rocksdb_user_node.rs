// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! RocksDB-based implementation of UserNodeCreator

use async_trait::async_trait;
use raisin_core::services::node_service::NodeService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{RepositoryManagementRepository, Storage};
use std::collections::HashMap;
use std::sync::Arc;

use super::email_to_node_name;
use super::user_node::UserNodeCreator;
use crate::RocksDBStorage;

/// RocksDB-based implementation of UserNodeCreator.
///
/// Creates user nodes directly in the `raisin:access_control` workspace
/// using RocksDB transactions.
pub struct RocksDBUserNodeCreator {
    storage: Arc<RocksDBStorage>,
}

impl RocksDBUserNodeCreator {
    /// Create a new RocksDBUserNodeCreator
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl UserNodeCreator for RocksDBUserNodeCreator {
    async fn create_user_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        identity_id: &str,
        email: &str,
        display_name: Option<&str>,
        default_roles: &[String],
    ) -> Result<String> {
        // Access control workspace where users are stored
        let workspace = "raisin:access_control";
        let users_path = "/users/internal";

        // Generate safe node name from email
        let node_name = email_to_node_name(email);
        let user_path = format!("{}/{}", users_path, node_name);

        // Get repository configuration to determine the default branch
        let default_branch = match self
            .storage
            .repository_management()
            .get_repository(tenant_id, repo_id)
            .await
        {
            Ok(Some(repo)) => repo.config.default_branch.clone(),
            _ => "main".to_string(), // Fallback if repo not found
        };

        tracing::info!(
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            identity_id = %identity_id,
            email = %email,
            user_path = %user_path,
            branch = %default_branch,
            "Creating user node for registered identity"
        );

        // Use NodeService to ensure initial_structure is created
        // This creates profile, inbox, outbox children defined in raisin:User NodeType
        // Use system auth context to bypass RLS - this is a privileged operation
        let node_service: NodeService<RocksDBStorage> = NodeService::new_with_context(
            self.storage.clone(),
            tenant_id.to_string(),
            repo_id.to_string(),
            default_branch,
            workspace.to_string(),
        )
        .with_auth(AuthContext::system());

        // Check if user node already exists
        if let Some(existing_node) = node_service.get_by_path(&user_path).await? {
            tracing::info!(
                user_node_id = %existing_node.id,
                user_path = %user_path,
                "User node already exists"
            );
            return Ok(existing_node.id);
        }

        // Build roles as references to role paths
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

        // Create user node - NodeService::add_deep_node handles:
        // 1. Auto-creating parent folders (/users, /users/internal)
        // 2. Creating the user node with validation
        // 3. Creating initial_structure children (profile, inbox, outbox)
        let user_node = Node {
            id: String::new(), // Will be auto-generated by add_deep_node
            node_type: "raisin:User".to_string(),
            name: node_name,
            path: String::new(), // Will be auto-generated by add_deep_node
            workspace: Some(workspace.to_string()),
            properties,
            ..Default::default()
        };

        let created_node = node_service.add_deep_node(users_path, user_node).await?;

        tracing::info!(
            user_node_id = %created_node.id,
            user_path = %created_node.path,
            "User node created successfully with initial_structure"
        );

        Ok(created_node.id)
    }
}
