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

//! Database admin user models and types.
//!
//! Admin users are service accounts stored per-tenant that provide access to:
//! - Admin Console UI
//! - CLI tools
//! - API programmatic access
//!
//! These are separate from workspace-level users (raisin:User NodeType) and are used
//! for database management and administrative tasks.

use serde::{Deserialize, Serialize};

use crate::timestamp::StorageTimestamp;

/// Access flags controlling what interfaces an admin user can access
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdminAccessFlags {
    /// Can login to the admin console web interface
    pub console_login: bool,
    /// Can use CLI tools
    pub cli_access: bool,
    /// Can access API programmatically
    pub api_access: bool,
    /// Can connect via PostgreSQL wire protocol (pgwire)
    #[serde(default)]
    pub pgwire_access: bool,
    /// Can impersonate regular users (raisin:User) for testing permissions
    #[serde(default)]
    pub can_impersonate: bool,
}

impl Default for AdminAccessFlags {
    fn default() -> Self {
        Self {
            console_login: true,
            cli_access: true,
            api_access: true,
            pgwire_access: false,
            can_impersonate: false,
        }
    }
}

/// Database admin user - a service account for database management
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DatabaseAdminUser {
    /// Unique identifier for the user
    pub user_id: String,
    /// Username for authentication (unique per tenant)
    pub username: String,
    /// Optional email address
    pub email: Option<String>,
    /// Bcrypt password hash
    pub password_hash: String,
    /// Tenant ID this user belongs to
    pub tenant_id: String,
    /// Access flags controlling what interfaces are available
    pub access_flags: AdminAccessFlags,
    /// Whether the user must change password on next login
    pub must_change_password: bool,
    /// When the user was created (i64 nanos in binary, RFC3339 in JSON)
    pub created_at: StorageTimestamp,
    /// Last successful login time (i64 nanos in binary, RFC3339 in JSON)
    pub last_login: Option<StorageTimestamp>,
    /// Whether the user account is active
    pub is_active: bool,
}

impl DatabaseAdminUser {
    /// Create a new admin user
    pub fn new(
        user_id: String,
        username: String,
        email: Option<String>,
        password_hash: String,
        tenant_id: String,
    ) -> Self {
        Self {
            user_id,
            username,
            email,
            password_hash,
            tenant_id,
            access_flags: AdminAccessFlags::default(),
            must_change_password: false,
            created_at: StorageTimestamp::now(),
            last_login: None,
            is_active: true,
        }
    }

    /// Create a superadmin user with all access flags enabled and password change required
    pub fn new_superadmin(
        user_id: String,
        username: String,
        password_hash: String,
        tenant_id: String,
    ) -> Self {
        Self {
            user_id,
            username,
            email: None,
            password_hash,
            tenant_id,
            access_flags: AdminAccessFlags::default(),
            must_change_password: true,
            created_at: StorageTimestamp::now(),
            last_login: None,
            is_active: true,
        }
    }

    /// Update the password hash
    pub fn update_password(&mut self, new_password_hash: String) {
        self.password_hash = new_password_hash;
        self.must_change_password = false;
    }

    /// Record a successful login
    pub fn record_login(&mut self) {
        self.last_login = Some(StorageTimestamp::now());
    }

    /// Check if user can access a specific interface
    pub fn can_access(&self, interface: AdminInterface) -> bool {
        if !self.is_active {
            return false;
        }

        match interface {
            AdminInterface::Console => self.access_flags.console_login,
            AdminInterface::Cli => self.access_flags.cli_access,
            AdminInterface::Api => self.access_flags.api_access,
            AdminInterface::Pgwire => self.access_flags.pgwire_access,
        }
    }
}

/// Interface types for admin access
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdminInterface {
    /// Admin console web interface
    Console,
    /// Command-line interface
    Cli,
    /// API/programmatic access
    Api,
    /// PostgreSQL wire protocol access
    Pgwire,
}

/// Request to create a new admin user
#[derive(Debug, Clone, Deserialize)]
pub struct CreateAdminUserRequest {
    pub username: String,
    pub email: Option<String>,
    pub password: String,
    pub access_flags: Option<AdminAccessFlags>,
}

/// Request to update an existing admin user
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAdminUserRequest {
    pub email: Option<String>,
    pub access_flags: Option<AdminAccessFlags>,
    pub is_active: Option<bool>,
}

/// Request to change password
#[derive(Debug, Clone, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// Response containing admin user info (without password hash)
#[derive(Debug, Clone, Serialize)]
pub struct AdminUserResponse {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub tenant_id: String,
    pub access_flags: AdminAccessFlags,
    pub must_change_password: bool,
    pub created_at: StorageTimestamp,
    pub last_login: Option<StorageTimestamp>,
    pub is_active: bool,
}

impl From<DatabaseAdminUser> for AdminUserResponse {
    fn from(user: DatabaseAdminUser) -> Self {
        Self {
            user_id: user.user_id,
            username: user.username,
            email: user.email,
            tenant_id: user.tenant_id,
            access_flags: user.access_flags,
            must_change_password: user.must_change_password,
            created_at: user.created_at,
            last_login: user.last_login,
            is_active: user.is_active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_user_creation() {
        let user = DatabaseAdminUser::new(
            "user1".to_string(),
            "admin".to_string(),
            Some("admin@example.com".to_string()),
            "hashed_password".to_string(),
            "default".to_string(),
        );

        assert_eq!(user.user_id, "user1");
        assert_eq!(user.username, "admin");
        assert!(user.is_active);
        assert!(!user.must_change_password);
        assert!(user.access_flags.console_login);
    }

    #[test]
    fn test_superadmin_creation() {
        let user = DatabaseAdminUser::new_superadmin(
            "admin1".to_string(),
            "admin".to_string(),
            "hashed_password".to_string(),
            "default".to_string(),
        );

        assert!(user.must_change_password);
        assert!(user.access_flags.console_login);
        assert!(user.access_flags.cli_access);
        assert!(user.access_flags.api_access);
    }

    #[test]
    fn test_can_access() {
        let mut user = DatabaseAdminUser::new(
            "user1".to_string(),
            "test".to_string(),
            None,
            "hash".to_string(),
            "default".to_string(),
        );

        user.access_flags.console_login = false;
        assert!(!user.can_access(AdminInterface::Console));
        assert!(user.can_access(AdminInterface::Api));

        user.is_active = false;
        assert!(!user.can_access(AdminInterface::Api));
    }

    #[test]
    fn test_update_password() {
        let mut user = DatabaseAdminUser::new_superadmin(
            "admin1".to_string(),
            "admin".to_string(),
            "old_hash".to_string(),
            "default".to_string(),
        );

        assert!(user.must_change_password);
        user.update_password("new_hash".to_string());
        assert_eq!(user.password_hash, "new_hash");
        assert!(!user.must_change_password);
    }
}
