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

//! Identity models for the pluggable authentication system.
//!
//! An Identity represents a unique person within a tenant, independent of
//! how they authenticate. A single identity can have multiple authentication
//! providers linked (e.g., password + Google + Okta).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::timestamp::StorageTimestamp;

/// A global identity within a tenant.
///
/// Identities are stored in the `raisin:system` workspace and represent
/// a unique person across all workspaces within the tenant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Identity {
    /// Unique identifier (UUID)
    pub identity_id: String,

    /// Tenant this identity belongs to
    pub tenant_id: String,

    /// Primary email address (unique within tenant)
    pub email: String,

    /// Whether the email has been verified
    pub email_verified: bool,

    /// Display name
    pub display_name: Option<String>,

    /// Avatar URL
    pub avatar_url: Option<String>,

    /// Whether the identity is active
    pub is_active: bool,

    /// Linked authentication providers
    pub linked_providers: Vec<LinkedProvider>,

    /// Local credentials (username/password)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_credentials: Option<LocalCredentials>,

    /// Custom metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, serde_json::Value>,

    /// When the identity was created
    pub created_at: StorageTimestamp,

    /// Last login time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_login_at: Option<StorageTimestamp>,

    /// Last time any profile field was updated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<StorageTimestamp>,
}

impl Identity {
    /// Create a new identity
    pub fn new(identity_id: String, tenant_id: String, email: String) -> Self {
        Self {
            identity_id,
            tenant_id,
            email,
            email_verified: false,
            display_name: None,
            avatar_url: None,
            is_active: true,
            linked_providers: Vec::new(),
            local_credentials: None,
            metadata: HashMap::new(),
            created_at: StorageTimestamp::now(),
            last_login_at: None,
            updated_at: None,
        }
    }

    /// Record a login event
    pub fn record_login(&mut self) {
        self.last_login_at = Some(StorageTimestamp::now());
    }

    /// Mark email as verified
    pub fn verify_email(&mut self) {
        self.email_verified = true;
        self.updated_at = Some(StorageTimestamp::now());
    }

    /// Check if a specific provider is linked
    pub fn has_provider(&self, strategy_id: &str) -> bool {
        self.linked_providers
            .iter()
            .any(|p| p.strategy_id == strategy_id)
    }

    /// Find a linked provider by strategy ID
    pub fn get_provider(&self, strategy_id: &str) -> Option<&LinkedProvider> {
        self.linked_providers
            .iter()
            .find(|p| p.strategy_id == strategy_id)
    }

    /// Link a new provider
    pub fn link_provider(&mut self, provider: LinkedProvider) {
        // Remove existing provider with same strategy_id if any
        self.linked_providers
            .retain(|p| p.strategy_id != provider.strategy_id);
        self.linked_providers.push(provider);
        self.updated_at = Some(StorageTimestamp::now());
    }

    /// Unlink a provider
    pub fn unlink_provider(&mut self, strategy_id: &str) -> bool {
        let before = self.linked_providers.len();
        self.linked_providers
            .retain(|p| p.strategy_id != strategy_id);
        let removed = self.linked_providers.len() < before;
        if removed {
            self.updated_at = Some(StorageTimestamp::now());
        }
        removed
    }

    /// Check if identity has local credentials
    pub fn has_local_credentials(&self) -> bool {
        self.local_credentials.is_some()
    }
}

/// A linked external authentication provider
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkedProvider {
    /// Strategy ID (e.g., "oidc:google", "oidc:okta", "saml:azure")
    pub strategy_id: String,

    /// External user ID from the provider
    pub external_id: String,

    /// Provider-specific claims/attributes
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub claims: HashMap<String, serde_json::Value>,

    /// When this provider was linked
    pub linked_at: StorageTimestamp,

    /// Last time authentication occurred via this provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_auth_at: Option<StorageTimestamp>,
}

impl LinkedProvider {
    /// Create a new linked provider
    pub fn new(strategy_id: String, external_id: String) -> Self {
        Self {
            strategy_id,
            external_id,
            claims: HashMap::new(),
            linked_at: StorageTimestamp::now(),
            last_auth_at: None,
        }
    }

    /// Record an authentication via this provider
    pub fn record_auth(&mut self) {
        self.last_auth_at = Some(StorageTimestamp::now());
    }

    /// Update claims from provider
    pub fn update_claims(&mut self, claims: HashMap<String, serde_json::Value>) {
        self.claims = claims;
    }
}

/// Local username/password credentials
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalCredentials {
    /// Optional username (email is primary identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Bcrypt password hash
    pub password_hash: String,

    /// Whether password change is required on next login
    pub must_change_password: bool,

    /// Failed login attempts counter
    #[serde(default)]
    pub failed_attempts: u32,

    /// Account locked until (if too many failed attempts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_until: Option<StorageTimestamp>,

    /// Password last changed
    pub password_changed_at: StorageTimestamp,
}

impl LocalCredentials {
    /// Create new local credentials
    pub fn new(password_hash: String) -> Self {
        Self {
            username: None,
            password_hash,
            must_change_password: false,
            failed_attempts: 0,
            locked_until: None,
            password_changed_at: StorageTimestamp::now(),
        }
    }

    /// Create credentials requiring password change
    pub fn new_with_change_required(password_hash: String) -> Self {
        Self {
            username: None,
            password_hash,
            must_change_password: true,
            failed_attempts: 0,
            locked_until: None,
            password_changed_at: StorageTimestamp::now(),
        }
    }

    /// Check if account is currently locked
    pub fn is_locked(&self) -> bool {
        if let Some(locked_until) = &self.locked_until {
            locked_until > &StorageTimestamp::now()
        } else {
            false
        }
    }

    /// Increment failed attempts and optionally lock account
    pub fn record_failed_attempt(&mut self, lockout_threshold: u32, lockout_duration_minutes: u64) {
        self.failed_attempts += 1;
        if self.failed_attempts >= lockout_threshold {
            let now = StorageTimestamp::now();
            let lockout_nanos =
                now.timestamp_nanos() + (lockout_duration_minutes as i64 * 60 * 1_000_000_000);
            self.locked_until = StorageTimestamp::from_nanos(lockout_nanos);
        }
    }

    /// Reset failed attempts (on successful login)
    pub fn reset_failed_attempts(&mut self) {
        self.failed_attempts = 0;
        self.locked_until = None;
    }

    /// Update password
    pub fn update_password(&mut self, new_hash: String) {
        self.password_hash = new_hash;
        self.must_change_password = false;
        self.password_changed_at = StorageTimestamp::now();
        self.reset_failed_attempts();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_creation() {
        let identity = Identity::new(
            "id-123".to_string(),
            "tenant-1".to_string(),
            "user@example.com".to_string(),
        );

        assert_eq!(identity.identity_id, "id-123");
        assert_eq!(identity.tenant_id, "tenant-1");
        assert_eq!(identity.email, "user@example.com");
        assert!(!identity.email_verified);
        assert!(identity.is_active);
        assert!(identity.linked_providers.is_empty());
    }

    #[test]
    fn test_link_provider() {
        let mut identity = Identity::new(
            "id-123".to_string(),
            "tenant-1".to_string(),
            "user@example.com".to_string(),
        );

        let provider = LinkedProvider::new("oidc:google".to_string(), "google-123".to_string());

        identity.link_provider(provider);

        assert!(identity.has_provider("oidc:google"));
        assert!(!identity.has_provider("oidc:okta"));
    }

    #[test]
    fn test_unlink_provider() {
        let mut identity = Identity::new(
            "id-123".to_string(),
            "tenant-1".to_string(),
            "user@example.com".to_string(),
        );

        let provider = LinkedProvider::new("oidc:google".to_string(), "google-123".to_string());
        identity.link_provider(provider);

        assert!(identity.has_provider("oidc:google"));

        let removed = identity.unlink_provider("oidc:google");
        assert!(removed);
        assert!(!identity.has_provider("oidc:google"));
    }

    #[test]
    fn test_local_credentials_lockout() {
        let mut creds = LocalCredentials::new("hash".to_string());

        // 5 failed attempts with 3 threshold
        for _ in 0..5 {
            creds.record_failed_attempt(3, 15);
        }

        assert!(creds.is_locked());
        assert_eq!(creds.failed_attempts, 5);

        // Reset on success
        creds.reset_failed_attempts();
        assert!(!creds.is_locked());
        assert_eq!(creds.failed_attempts, 0);
    }
}
