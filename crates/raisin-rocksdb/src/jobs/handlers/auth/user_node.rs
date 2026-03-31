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

//! User node creation handler and types

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

use super::email_to_node_name;

/// Callback trait for creating user nodes in the access_control workspace
#[async_trait]
pub trait UserNodeCreator: Send + Sync {
    /// Create a user node in the repository's access_control workspace
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `identity_id` - Identity ID of the user
    /// * `email` - User's email address
    /// * `display_name` - Optional display name
    /// * `default_roles` - Roles to assign to the user
    ///
    /// # Returns
    ///
    /// Returns `Ok(user_node_id)` on success, or an error if creation fails.
    async fn create_user_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        identity_id: &str,
        email: &str,
        display_name: Option<&str>,
        default_roles: &[String],
    ) -> Result<String>;
}

/// Data for the create user node job
#[derive(Debug, Clone)]
pub struct CreateUserNodeJobData {
    /// Identity ID of the user
    pub identity_id: String,
    /// Repository ID where the user registered
    pub repo_id: String,
    /// User's email address
    pub email: String,
    /// Optional display name
    pub display_name: Option<String>,
    /// Default roles to assign
    pub default_roles: Vec<String>,
}

impl CreateUserNodeJobData {
    /// Create new job data
    pub fn new(
        identity_id: impl Into<String>,
        repo_id: impl Into<String>,
        email: impl Into<String>,
        display_name: Option<String>,
        default_roles: Vec<String>,
    ) -> Self {
        Self {
            identity_id: identity_id.into(),
            repo_id: repo_id.into(),
            email: email.into(),
            display_name,
            default_roles,
        }
    }

    /// Convert to metadata for job context
    pub fn to_metadata(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "identity_id".to_string(),
            serde_json::json!(self.identity_id),
        );
        metadata.insert("repo_id".to_string(), serde_json::json!(self.repo_id));
        metadata.insert("email".to_string(), serde_json::json!(self.email));
        if let Some(ref name) = self.display_name {
            metadata.insert("display_name".to_string(), serde_json::json!(name));
        }
        metadata.insert(
            "default_roles".to_string(),
            serde_json::json!(self.default_roles),
        );
        metadata
    }

    /// Parse from job context metadata
    pub fn from_metadata(
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Option<Self> {
        let identity_id = metadata.get("identity_id")?.as_str()?.to_string();
        let repo_id = metadata.get("repo_id")?.as_str()?.to_string();
        let email = metadata.get("email")?.as_str()?.to_string();
        let display_name = metadata
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let default_roles = metadata
            .get("default_roles")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_else(|| vec!["viewer".to_string()]);

        Some(Self {
            identity_id,
            repo_id,
            email,
            display_name,
            default_roles,
        })
    }
}

/// Result of creating a user node
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CreateUserNodeResult {
    /// Whether the operation was successful
    pub success: bool,
    /// Created user node ID
    pub user_node_id: Option<String>,
    /// User node path
    pub user_path: Option<String>,
    /// Whether the user already existed
    pub already_exists: bool,
    /// Error message if failed
    pub error: Option<String>,
}

impl CreateUserNodeResult {
    /// Create a success result
    pub fn success(user_node_id: String, user_path: String) -> Self {
        Self {
            success: true,
            user_node_id: Some(user_node_id),
            user_path: Some(user_path),
            already_exists: false,
            error: None,
        }
    }

    /// Create an already-exists result
    pub fn already_exists(user_node_id: String, user_path: String) -> Self {
        Self {
            success: true,
            user_node_id: Some(user_node_id),
            user_path: Some(user_path),
            already_exists: true,
            error: None,
        }
    }

    /// Create a failure result
    pub fn failure(error: String) -> Self {
        Self {
            success: false,
            user_node_id: None,
            user_path: None,
            already_exists: false,
            error: Some(error),
        }
    }
}

/// Handler for creating user nodes on registration
pub struct AuthCreateUserNodeHandler<C: UserNodeCreator> {
    node_creator: Arc<C>,
}

impl<C: UserNodeCreator> AuthCreateUserNodeHandler<C> {
    /// Create a new create user node handler
    pub fn new(node_creator: Arc<C>) -> Self {
        Self { node_creator }
    }

    /// Handle create user node job
    pub async fn handle(
        &self,
        job: &JobInfo,
        _context: &JobContext,
    ) -> Result<CreateUserNodeResult> {
        // Verify job type and extract parameters
        let (identity_id, repo_id, email, display_name, default_roles) = match &job.job_type {
            JobType::AuthCreateUserNode {
                identity_id,
                repo_id,
                email,
                display_name,
                default_roles,
            } => (
                identity_id.clone(),
                repo_id.clone(),
                email.clone(),
                display_name.clone(),
                default_roles.clone(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected AuthCreateUserNode job type".to_string(),
                ))
            }
        };

        let tenant_id = &_context.tenant_id;

        tracing::info!(
            job_id = %job.id,
            tenant_id = %tenant_id,
            repo_id = %repo_id,
            identity_id = %identity_id,
            email = %email,
            "Creating user node for registered identity"
        );

        // Create the user node
        match self
            .node_creator
            .create_user_node(
                tenant_id,
                &repo_id,
                &identity_id,
                &email,
                display_name.as_deref(),
                &default_roles,
            )
            .await
        {
            Ok(user_node_id) => {
                let user_path = format!("/users/internal/{}", email_to_node_name(&email));

                tracing::info!(
                    job_id = %job.id,
                    user_node_id = %user_node_id,
                    user_path = %user_path,
                    "User node created successfully"
                );

                Ok(CreateUserNodeResult::success(user_node_id, user_path))
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to create user node"
                );

                Ok(CreateUserNodeResult::failure(e.to_string()))
            }
        }
    }
}
