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

//! Workspace access models for the authentication system.
//!
//! Links global identities to workspace-specific users (raisin:User nodes).

use serde::{Deserialize, Serialize};

use crate::timestamp::StorageTimestamp;

/// Workspace access record linking an Identity to a workspace-specific user.
///
/// When an identity gains access to a workspace:
/// 1. A `WorkspaceAccess` record is created linking the identity to the workspace
/// 2. A `raisin:User` node is created in the workspace's `raisin:access_control`
/// 3. The `user_node_id` references this workspace-specific user node
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceAccess {
    /// Identity ID (global user)
    pub identity_id: String,

    /// Tenant ID
    pub tenant_id: String,

    /// Repository/workspace ID
    pub repo_id: String,

    /// Workspace-specific user node ID (raisin:User in raisin:access_control)
    pub user_node_id: String,

    /// Current access status
    pub status: AccessStatus,

    /// When access was granted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<StorageTimestamp>,

    /// Who granted access (admin identity_id)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub granted_by: Option<String>,

    /// When access was requested (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_at: Option<StorageTimestamp>,

    /// When access was last modified
    pub updated_at: StorageTimestamp,

    /// Roles assigned to the user in this workspace
    #[serde(default)]
    pub roles: Vec<String>,

    /// Notes/reason for access decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl WorkspaceAccess {
    /// Create a new pending access request
    pub fn new_request(identity_id: String, tenant_id: String, repo_id: String) -> Self {
        let now = StorageTimestamp::now();
        Self {
            identity_id,
            tenant_id,
            repo_id,
            user_node_id: String::new(), // Will be set when approved
            status: AccessStatus::Pending,
            granted_at: None,
            granted_by: None,
            requested_at: Some(now),
            updated_at: now,
            roles: Vec::new(),
            notes: None,
        }
    }

    /// Create a new active access (direct grant or invitation accepted)
    pub fn new_active(
        identity_id: String,
        tenant_id: String,
        repo_id: String,
        user_node_id: String,
        granted_by: Option<String>,
        roles: Vec<String>,
    ) -> Self {
        let now = StorageTimestamp::now();
        Self {
            identity_id,
            tenant_id,
            repo_id,
            user_node_id,
            status: AccessStatus::Active,
            granted_at: Some(now),
            granted_by,
            requested_at: None,
            updated_at: now,
            roles,
            notes: None,
        }
    }

    /// Create a new invitation
    pub fn new_invitation(
        identity_id: String,
        tenant_id: String,
        repo_id: String,
        invited_by: String,
        roles: Vec<String>,
    ) -> Self {
        let now = StorageTimestamp::now();
        Self {
            identity_id,
            tenant_id,
            repo_id,
            user_node_id: String::new(), // Will be set when accepted
            status: AccessStatus::Invited,
            granted_at: None,
            granted_by: Some(invited_by),
            requested_at: None,
            updated_at: now,
            roles,
            notes: None,
        }
    }

    /// Check if access is currently active
    pub fn is_active(&self) -> bool {
        self.status == AccessStatus::Active
    }

    /// Approve a pending request
    pub fn approve(&mut self, user_node_id: String, approved_by: String) {
        self.status = AccessStatus::Active;
        self.user_node_id = user_node_id;
        self.granted_at = Some(StorageTimestamp::now());
        self.granted_by = Some(approved_by);
        self.updated_at = StorageTimestamp::now();
    }

    /// Deny a pending request
    pub fn deny(&mut self, denied_by: String, reason: Option<String>) {
        self.status = AccessStatus::Denied;
        self.granted_by = Some(denied_by);
        self.notes = reason;
        self.updated_at = StorageTimestamp::now();
    }

    /// Revoke active access
    pub fn revoke(&mut self, revoked_by: String, reason: Option<String>) {
        self.status = AccessStatus::Revoked;
        self.granted_by = Some(revoked_by);
        self.notes = reason;
        self.updated_at = StorageTimestamp::now();
    }

    /// Accept an invitation
    pub fn accept_invitation(&mut self, user_node_id: String) {
        if self.status == AccessStatus::Invited {
            self.status = AccessStatus::Active;
            self.user_node_id = user_node_id;
            self.granted_at = Some(StorageTimestamp::now());
            self.updated_at = StorageTimestamp::now();
        }
    }

    /// Decline an invitation
    pub fn decline_invitation(&mut self) {
        if self.status == AccessStatus::Invited {
            self.status = AccessStatus::Declined;
            self.updated_at = StorageTimestamp::now();
        }
    }
}

/// Status of workspace access
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    /// Access is active and user can access the workspace
    Active,

    /// User has requested access, pending approval
    Pending,

    /// User has been invited but hasn't accepted yet
    Invited,

    /// Access request was denied
    Denied,

    /// Access was revoked after being granted
    Revoked,

    /// User declined the invitation
    Declined,

    /// Access is suspended (temporary)
    Suspended,
}

impl AccessStatus {
    /// Check if this status allows workspace access
    pub fn allows_access(&self) -> bool {
        matches!(self, AccessStatus::Active)
    }

    /// Check if this is a final state (no further transitions expected)
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            AccessStatus::Denied | AccessStatus::Revoked | AccessStatus::Declined
        )
    }
}

/// Settings for workspace access control
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessSettings {
    /// Allow users to request access
    pub allow_access_requests: bool,

    /// Require approval for access requests (vs auto-approve)
    pub require_approval: bool,

    /// Allow invitations
    pub allow_invitations: bool,

    /// Default roles for new users (if auto-approved)
    #[serde(default)]
    pub default_roles: Vec<String>,

    /// Maximum pending requests per workspace
    #[serde(default = "default_max_pending")]
    pub max_pending_requests: u32,

    /// Invitation expiration in days
    #[serde(default = "default_invitation_expiry")]
    pub invitation_expiry_days: u32,
}

fn default_max_pending() -> u32 {
    100
}

fn default_invitation_expiry() -> u32 {
    7
}

impl Default for AccessSettings {
    fn default() -> Self {
        Self {
            allow_access_requests: true,
            require_approval: true,
            allow_invitations: true,
            default_roles: vec!["viewer".to_string()],
            max_pending_requests: default_max_pending(),
            invitation_expiry_days: default_invitation_expiry(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_request_flow() {
        let mut access = WorkspaceAccess::new_request(
            "id-123".to_string(),
            "tenant-1".to_string(),
            "repo-1".to_string(),
        );

        assert_eq!(access.status, AccessStatus::Pending);
        assert!(!access.is_active());
        assert!(access.requested_at.is_some());

        // Approve
        access.approve("user-node-1".to_string(), "admin-1".to_string());

        assert_eq!(access.status, AccessStatus::Active);
        assert!(access.is_active());
        assert_eq!(access.user_node_id, "user-node-1");
        assert!(access.granted_at.is_some());
    }

    #[test]
    fn test_invitation_flow() {
        let mut access = WorkspaceAccess::new_invitation(
            "id-123".to_string(),
            "tenant-1".to_string(),
            "repo-1".to_string(),
            "admin-1".to_string(),
            vec!["editor".to_string()],
        );

        assert_eq!(access.status, AccessStatus::Invited);
        assert!(!access.is_active());

        // Accept invitation
        access.accept_invitation("user-node-1".to_string());

        assert_eq!(access.status, AccessStatus::Active);
        assert!(access.is_active());
    }

    #[test]
    fn test_revoke_access() {
        let mut access = WorkspaceAccess::new_active(
            "id-123".to_string(),
            "tenant-1".to_string(),
            "repo-1".to_string(),
            "user-node-1".to_string(),
            Some("admin-1".to_string()),
            vec!["editor".to_string()],
        );

        assert!(access.is_active());

        access.revoke("admin-1".to_string(), Some("Policy violation".to_string()));

        assert_eq!(access.status, AccessStatus::Revoked);
        assert!(!access.is_active());
        assert_eq!(access.notes, Some("Policy violation".to_string()));
    }

    #[test]
    fn test_access_status() {
        assert!(AccessStatus::Active.allows_access());
        assert!(!AccessStatus::Pending.allows_access());
        assert!(!AccessStatus::Invited.allows_access());

        assert!(AccessStatus::Denied.is_final());
        assert!(AccessStatus::Revoked.is_final());
        assert!(!AccessStatus::Pending.is_final());
    }
}
