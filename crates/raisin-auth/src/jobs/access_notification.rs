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

//! Access notification job helpers.
//!
//! This module provides helpers for creating access notification jobs
//! that notify users about workspace access changes (granted, revoked, etc.).

use super::AccessNotificationType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Access notification job data stored in JobContext
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessNotificationJobData {
    /// The identity ID of the user to notify
    pub identity_id: String,
    /// The email address to send notification to
    pub email: String,
    /// The repository/workspace ID
    pub repo_id: String,
    /// The workspace name (for display in notification)
    pub workspace_name: String,
    /// Type of access notification
    pub notification_type: String,
    /// ID of the user who performed the action (if applicable)
    pub actor_id: Option<String>,
    /// Name of the user who performed the action
    pub actor_name: Option<String>,
    /// Roles granted (for grant/invite notifications)
    pub roles: Vec<String>,
    /// Optional message from the actor
    pub message: Option<String>,
    /// Base URL for links in the notification
    pub base_url: String,
    /// Optional custom email template name
    pub template: Option<String>,
}

impl AccessNotificationJobData {
    /// Create new access notification job data
    pub fn new(
        identity_id: impl Into<String>,
        email: impl Into<String>,
        repo_id: impl Into<String>,
        workspace_name: impl Into<String>,
        notification_type: AccessNotificationType,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            identity_id: identity_id.into(),
            email: email.into(),
            repo_id: repo_id.into(),
            workspace_name: workspace_name.into(),
            notification_type: notification_type.as_str().to_string(),
            actor_id: None,
            actor_name: None,
            roles: Vec::new(),
            message: None,
            base_url: base_url.into(),
            template: None,
        }
    }

    /// Set the actor who performed the action
    pub fn with_actor(
        mut self,
        actor_id: impl Into<String>,
        actor_name: impl Into<String>,
    ) -> Self {
        self.actor_id = Some(actor_id.into());
        self.actor_name = Some(actor_name.into());
        self
    }

    /// Set the roles granted
    pub fn with_roles(mut self, roles: Vec<String>) -> Self {
        self.roles = roles;
        self
    }

    /// Set a custom message
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Set custom email template
    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    /// Get the notification type enum
    pub fn get_notification_type(&self) -> Option<AccessNotificationType> {
        AccessNotificationType::parse(&self.notification_type)
    }

    /// Build the workspace URL
    pub fn build_workspace_url(&self) -> String {
        format!("{}/workspaces/{}", self.base_url, self.repo_id)
    }

    /// Convert to metadata HashMap for JobContext
    pub fn to_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert(
            "identity_id".to_string(),
            serde_json::json!(self.identity_id),
        );
        map.insert("email".to_string(), serde_json::json!(self.email));
        map.insert("repo_id".to_string(), serde_json::json!(self.repo_id));
        map.insert(
            "workspace_name".to_string(),
            serde_json::json!(self.workspace_name),
        );
        map.insert(
            "notification_type".to_string(),
            serde_json::json!(self.notification_type),
        );
        if let Some(actor_id) = &self.actor_id {
            map.insert("actor_id".to_string(), serde_json::json!(actor_id));
        }
        if let Some(actor_name) = &self.actor_name {
            map.insert("actor_name".to_string(), serde_json::json!(actor_name));
        }
        if !self.roles.is_empty() {
            map.insert("roles".to_string(), serde_json::json!(self.roles));
        }
        if let Some(message) = &self.message {
            map.insert("message".to_string(), serde_json::json!(message));
        }
        map.insert("base_url".to_string(), serde_json::json!(self.base_url));
        map.insert(
            "workspace_url".to_string(),
            serde_json::json!(self.build_workspace_url()),
        );
        if let Some(template) = &self.template {
            map.insert("template".to_string(), serde_json::json!(template));
        }
        map
    }

    /// Parse from metadata HashMap
    pub fn from_metadata(metadata: &HashMap<String, serde_json::Value>) -> Option<Self> {
        Some(Self {
            identity_id: metadata.get("identity_id")?.as_str()?.to_string(),
            email: metadata.get("email")?.as_str()?.to_string(),
            repo_id: metadata.get("repo_id")?.as_str()?.to_string(),
            workspace_name: metadata.get("workspace_name")?.as_str()?.to_string(),
            notification_type: metadata.get("notification_type")?.as_str()?.to_string(),
            actor_id: metadata
                .get("actor_id")
                .and_then(|v| v.as_str())
                .map(String::from),
            actor_name: metadata
                .get("actor_name")
                .and_then(|v| v.as_str())
                .map(String::from),
            roles: metadata
                .get("roles")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            message: metadata
                .get("message")
                .and_then(|v| v.as_str())
                .map(String::from),
            base_url: metadata.get("base_url")?.as_str()?.to_string(),
            template: metadata
                .get("template")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }
}

/// Email template for access granted notification (plain text)
pub const ACCESS_GRANTED_TEMPLATE_TEXT: &str = r#"
Hello,

You have been granted access to the workspace "{workspace_name}".

{actor_message}

Access the workspace here:
{workspace_url}

{roles_message}

Best regards,
The Team
"#;

/// Email template for access granted notification (HTML)
pub const ACCESS_GRANTED_TEMPLATE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Access Granted</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h1 style="color: #16a34a;">Access Granted</h1>
        <p>Hello,</p>
        <p>You have been granted access to the workspace <strong>"{workspace_name}"</strong>.</p>
        {actor_message}
        <p>
            <a href="{workspace_url}"
               style="display: inline-block; background-color: #16a34a; color: white;
                      padding: 12px 24px; text-decoration: none; border-radius: 6px;
                      font-weight: bold;">
                Open Workspace
            </a>
        </p>
        {roles_message}
        <p style="color: #999; font-size: 12px; margin-top: 30px;">
            If you didn't expect this access, please contact your administrator.
        </p>
    </div>
</body>
</html>
"#;

/// Email template for access revoked notification (plain text)
pub const ACCESS_REVOKED_TEMPLATE_TEXT: &str = r#"
Hello,

Your access to the workspace "{workspace_name}" has been revoked.

{actor_message}

If you believe this was a mistake, please contact your administrator.

Best regards,
The Team
"#;

/// Email template for access revoked notification (HTML)
pub const ACCESS_REVOKED_TEMPLATE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Access Revoked</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h1 style="color: #dc2626;">Access Revoked</h1>
        <p>Hello,</p>
        <p>Your access to the workspace <strong>"{workspace_name}"</strong> has been revoked.</p>
        {actor_message}
        <p style="color: #999; font-size: 12px; margin-top: 30px;">
            If you believe this was a mistake, please contact your administrator.
        </p>
    </div>
</body>
</html>
"#;

/// Email template for invitation notification (plain text)
pub const INVITATION_TEMPLATE_TEXT: &str = r#"
Hello,

You have been invited to join the workspace "{workspace_name}".

{actor_message}

Accept the invitation and access the workspace here:
{workspace_url}

{roles_message}

This invitation will expire in 7 days.

Best regards,
The Team
"#;

/// Email template for invitation notification (HTML)
pub const INVITATION_TEMPLATE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Workspace Invitation</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h1 style="color: #2563eb;">You're Invited!</h1>
        <p>Hello,</p>
        <p>You have been invited to join the workspace <strong>"{workspace_name}"</strong>.</p>
        {actor_message}
        <p>
            <a href="{workspace_url}"
               style="display: inline-block; background-color: #2563eb; color: white;
                      padding: 12px 24px; text-decoration: none; border-radius: 6px;
                      font-weight: bold;">
                Accept Invitation
            </a>
        </p>
        {roles_message}
        <p style="color: #666; font-size: 14px;">
            This invitation will expire in 7 days.
        </p>
        <p style="color: #999; font-size: 12px; margin-top: 30px;">
            If you didn't expect this invitation, you can safely ignore this email.
        </p>
    </div>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_notification_job_data() {
        let data = AccessNotificationJobData::new(
            "identity-123",
            "user@example.com",
            "repo-456",
            "My Workspace",
            AccessNotificationType::Granted,
            "https://app.example.com",
        )
        .with_actor("admin-789", "Admin User")
        .with_roles(vec!["editor".to_string(), "reviewer".to_string()])
        .with_message("Welcome to the team!");

        assert_eq!(data.identity_id, "identity-123");
        assert_eq!(data.email, "user@example.com");
        assert_eq!(data.repo_id, "repo-456");
        assert_eq!(data.workspace_name, "My Workspace");
        assert_eq!(
            data.get_notification_type(),
            Some(AccessNotificationType::Granted)
        );
        assert_eq!(data.actor_id, Some("admin-789".to_string()));
        assert_eq!(data.roles, vec!["editor", "reviewer"]);
        assert_eq!(
            data.build_workspace_url(),
            "https://app.example.com/workspaces/repo-456"
        );
    }

    #[test]
    fn test_metadata_roundtrip() {
        let original = AccessNotificationJobData::new(
            "identity-123",
            "user@example.com",
            "repo-456",
            "My Workspace",
            AccessNotificationType::Invited,
            "https://app.example.com",
        )
        .with_actor("admin-789", "Admin User")
        .with_roles(vec!["viewer".to_string()])
        .with_template("custom_template");

        let metadata = original.to_metadata();
        let restored = AccessNotificationJobData::from_metadata(&metadata).unwrap();

        assert_eq!(restored.identity_id, original.identity_id);
        assert_eq!(restored.email, original.email);
        assert_eq!(restored.repo_id, original.repo_id);
        assert_eq!(restored.workspace_name, original.workspace_name);
        assert_eq!(restored.notification_type, original.notification_type);
        assert_eq!(restored.actor_id, original.actor_id);
        assert_eq!(restored.actor_name, original.actor_name);
        assert_eq!(restored.roles, original.roles);
        assert_eq!(restored.template, original.template);
    }

    #[test]
    fn test_all_notification_types() {
        let types = vec![
            AccessNotificationType::Granted,
            AccessNotificationType::Revoked,
            AccessNotificationType::RequestApproved,
            AccessNotificationType::RequestDenied,
            AccessNotificationType::Invited,
        ];

        for t in types {
            let data = AccessNotificationJobData::new(
                "id",
                "email@test.com",
                "repo",
                "workspace",
                t.clone(),
                "https://example.com",
            );
            assert_eq!(data.get_notification_type(), Some(t));
        }
    }
}
