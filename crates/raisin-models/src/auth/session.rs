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

//! Session and token models for the authentication system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::timestamp::StorageTimestamp;

/// An active authentication session.
///
/// Sessions are created on successful authentication and track
/// the user's login state, including refresh token information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    /// Unique session identifier
    pub session_id: String,

    /// Tenant this session belongs to
    pub tenant_id: String,

    /// Identity ID this session belongs to
    pub identity_id: String,

    /// Strategy used for authentication
    pub strategy_id: String,

    /// When the session was created
    pub created_at: StorageTimestamp,

    /// When the session expires
    pub expires_at: StorageTimestamp,

    /// Last activity time (updated on refresh)
    pub last_activity_at: StorageTimestamp,

    /// Client information
    pub client_info: ClientInfo,

    /// Whether this session has been revoked
    pub revoked: bool,

    /// Reason for revocation (if revoked)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,

    /// When the session was revoked
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<StorageTimestamp>,

    /// Hash of the current refresh token (for rotation validation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token_hash: Option<String>,

    /// Token family ID (for detecting refresh token reuse)
    pub token_family: String,

    /// Current generation in the token family (incremented on each refresh)
    pub token_generation: u32,
}

impl Session {
    /// Create a new session
    pub fn new(
        session_id: String,
        tenant_id: String,
        identity_id: String,
        strategy_id: String,
        token_family: String,
        expires_at: StorageTimestamp,
    ) -> Self {
        let now = StorageTimestamp::now();
        Self {
            session_id,
            tenant_id,
            identity_id,
            strategy_id,
            created_at: now,
            expires_at,
            last_activity_at: now,
            client_info: ClientInfo::default(),
            revoked: false,
            revoked_reason: None,
            revoked_at: None,
            refresh_token_hash: None,
            token_family,
            token_generation: 0,
        }
    }

    /// Check if session is valid (not revoked and not expired)
    pub fn is_valid(&self) -> bool {
        !self.revoked && self.expires_at > StorageTimestamp::now()
    }

    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        self.expires_at <= StorageTimestamp::now()
    }

    /// Revoke the session
    pub fn revoke(&mut self, reason: impl Into<String>) {
        self.revoked = true;
        self.revoked_reason = Some(reason.into());
        self.revoked_at = Some(StorageTimestamp::now());
    }

    /// Update activity timestamp
    pub fn touch(&mut self) {
        self.last_activity_at = StorageTimestamp::now();
    }

    /// Rotate refresh token (increment generation)
    pub fn rotate_refresh_token(&mut self, new_hash: String) {
        self.refresh_token_hash = Some(new_hash);
        self.token_generation += 1;
        self.touch();
    }
}

/// Client information associated with a session
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ClientInfo {
    /// Client IP address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,

    /// User agent string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,

    /// Device type (desktop, mobile, tablet)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,

    /// Browser name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,

    /// Operating system
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,

    /// Geographic location (country/city)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// One-time token for various purposes (magic links, password reset, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OneTimeToken {
    /// Token identifier
    pub token_id: String,

    /// Tenant this token belongs to
    pub tenant_id: String,

    /// Hash of the actual token value (never store plaintext)
    pub token_hash: String,

    /// Token prefix for identification (first 8 chars)
    pub token_prefix: String,

    /// Purpose of this token
    pub purpose: TokenPurpose,

    /// Identity ID this token is for (if known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_id: Option<String>,

    /// Email this token was sent to
    pub email: String,

    /// When the token was created
    pub created_at: StorageTimestamp,

    /// When the token expires
    pub expires_at: StorageTimestamp,

    /// Whether the token has been used
    pub used: bool,

    /// When the token was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_at: Option<StorageTimestamp>,

    /// Additional context data
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub context: HashMap<String, serde_json::Value>,
}

impl OneTimeToken {
    /// Create a new one-time token
    pub fn new(
        token_id: String,
        tenant_id: String,
        token_hash: String,
        token_prefix: String,
        purpose: TokenPurpose,
        email: String,
        expires_at: StorageTimestamp,
    ) -> Self {
        Self {
            token_id,
            tenant_id,
            token_hash,
            token_prefix,
            purpose,
            identity_id: None,
            email,
            created_at: StorageTimestamp::now(),
            expires_at,
            used: false,
            used_at: None,
            context: HashMap::new(),
        }
    }

    /// Check if token is valid (not used and not expired)
    pub fn is_valid(&self) -> bool {
        !self.used && self.expires_at > StorageTimestamp::now()
    }

    /// Mark token as used
    pub fn mark_used(&mut self) {
        self.used = true;
        self.used_at = Some(StorageTimestamp::now());
    }
}

/// Purpose of a one-time token
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenPurpose {
    /// Magic link for passwordless login
    MagicLink,

    /// Password reset token
    PasswordReset,

    /// Email verification
    EmailVerification,

    /// Account activation
    AccountActivation,

    /// Workspace invitation
    WorkspaceInvite {
        workspace_id: String,
        #[serde(default)]
        roles: Vec<String>,
    },

    /// API access token (long-lived)
    ApiAccess {
        /// Expiration in seconds (0 = never)
        expires_in_seconds: u64,
        /// Scopes granted to this token
        #[serde(default)]
        scopes: Vec<String>,
    },

    /// CLI login token
    CliLogin,

    /// Custom purpose
    Custom { name: String },
}

impl TokenPurpose {
    /// Get default expiration in minutes for each purpose
    pub fn default_expiration_minutes(&self) -> u64 {
        match self {
            TokenPurpose::MagicLink => 15,
            TokenPurpose::PasswordReset => 60,
            TokenPurpose::EmailVerification => 24 * 60, // 24 hours
            TokenPurpose::AccountActivation => 7 * 24 * 60, // 7 days
            TokenPurpose::WorkspaceInvite { .. } => 7 * 24 * 60, // 7 days
            TokenPurpose::ApiAccess {
                expires_in_seconds, ..
            } => {
                if *expires_in_seconds == 0 {
                    365 * 24 * 60 // 1 year for "never" expiring
                } else {
                    expires_in_seconds / 60
                }
            }
            TokenPurpose::CliLogin => 10,
            TokenPurpose::Custom { .. } => 60,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let expires = StorageTimestamp::from_nanos(
            StorageTimestamp::now().timestamp_nanos() + 24 * 60 * 60 * 1_000_000_000,
        )
        .unwrap();

        let session = Session::new(
            "sess-123".to_string(),
            "tenant-1".to_string(),
            "id-123".to_string(),
            "local".to_string(),
            "family-1".to_string(),
            expires,
        );

        assert!(session.is_valid());
        assert!(!session.is_expired());
        assert!(!session.revoked);
    }

    #[test]
    fn test_session_revocation() {
        let expires = StorageTimestamp::from_nanos(
            StorageTimestamp::now().timestamp_nanos() + 24 * 60 * 60 * 1_000_000_000,
        )
        .unwrap();

        let mut session = Session::new(
            "sess-123".to_string(),
            "tenant-1".to_string(),
            "id-123".to_string(),
            "local".to_string(),
            "family-1".to_string(),
            expires,
        );

        session.revoke("user logout");

        assert!(!session.is_valid());
        assert!(session.revoked);
        assert_eq!(session.revoked_reason, Some("user logout".to_string()));
    }

    #[test]
    fn test_token_purpose_expiration() {
        assert_eq!(TokenPurpose::MagicLink.default_expiration_minutes(), 15);
        assert_eq!(TokenPurpose::PasswordReset.default_expiration_minutes(), 60);

        let api_access = TokenPurpose::ApiAccess {
            expires_in_seconds: 3600,
            scopes: vec![],
        };
        assert_eq!(api_access.default_expiration_minutes(), 60);
    }

    #[test]
    fn test_one_time_token() {
        let expires = StorageTimestamp::from_nanos(
            StorageTimestamp::now().timestamp_nanos() + 15 * 60 * 1_000_000_000,
        )
        .unwrap();

        let mut token = OneTimeToken::new(
            "tok-123".to_string(),
            "tenant-1".to_string(),
            "hash".to_string(),
            "abc12345".to_string(),
            TokenPurpose::MagicLink,
            "user@example.com".to_string(),
            expires,
        );

        assert!(token.is_valid());

        token.mark_used();

        assert!(!token.is_valid());
        assert!(token.used);
    }
}
