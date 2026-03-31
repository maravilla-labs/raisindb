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

//! JWT claims for the authentication system.
//!
//! Uses a "lean JWT" approach where workspace-specific permissions are
//! resolved via hot LRU cache rather than embedded in the token.

use serde::{Deserialize, Serialize};

/// JWT claims for authenticated users.
///
/// This is a "lean" JWT that contains only identity and global flags.
/// Workspace-specific permissions are resolved at runtime via a hot
/// LRU cache keyed by (session_id, workspace_id).
///
/// # Why Lean JWT?
///
/// Embedding workspace permissions in the JWT causes:
/// - Token size bloat (>4KB with many workspaces)
/// - Stale permissions until token refresh
/// - HTTP header size limit issues
///
/// Instead, we:
/// 1. Store only `identity_id`, `session_id`, and `global_flags` in JWT
/// 2. Pass active workspace via `X-Raisin-Workspace` header
/// 3. Cache permissions in LRU cache with 5-min TTL
/// 4. Invalidate cache on role/permission changes via EventBus
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuthClaims {
    /// Subject - the identity_id
    pub sub: String,

    /// User's email
    pub email: String,

    /// Tenant ID
    pub tenant_id: String,

    /// Repository ID (for repo-scoped authentication)
    /// Only set when authenticating via /auth/{repo}/login or /auth/{repo}/register
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,

    /// User's home path (the raisin:User node path in raisin:access_control workspace)
    /// Available for fast access without database lookup.
    /// Used for path-based access control (e.g., `node.path.descendantOf($auth.home)`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,

    /// Session ID (for session management and cache keying)
    pub sid: String,

    /// Strategy used for authentication
    pub auth_strategy: String,

    /// Unix timestamp of actual authentication (for sudo mode)
    ///
    /// Used to require re-authentication for sensitive operations.
    /// If `now - auth_time > threshold`, user must re-authenticate.
    pub auth_time: i64,

    /// Global flags (tenant-wide, not workspace-specific)
    pub global_flags: GlobalFlags,

    /// Token type
    pub token_type: TokenType,

    /// Expiration time (Unix timestamp)
    pub exp: i64,

    /// Issued at time (Unix timestamp)
    pub iat: i64,

    /// Not valid before (Unix timestamp)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<i64>,

    /// JWT ID (for revocation tracking)
    pub jti: String,

    /// Issuer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,

    /// Audience
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
}

impl AuthClaims {
    /// Check if the token is expired
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.exp <= now
    }

    /// Check if re-authentication is required for sensitive operations
    pub fn requires_reauth(&self, max_age_seconds: i64) -> bool {
        let now = chrono::Utc::now().timestamp();
        now - self.auth_time > max_age_seconds
    }

    /// Check if this is a tenant admin
    pub fn is_tenant_admin(&self) -> bool {
        self.global_flags.is_tenant_admin
    }

    /// Get the identity ID
    pub fn identity_id(&self) -> &str {
        &self.sub
    }

    /// Get the session ID
    pub fn session_id(&self) -> &str {
        &self.sid
    }
}

/// Global flags that apply tenant-wide (not workspace-specific)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GlobalFlags {
    /// Whether user is a tenant administrator
    pub is_tenant_admin: bool,

    /// Whether email has been verified
    pub email_verified: bool,

    /// Whether password change is required
    #[serde(default)]
    pub must_change_password: bool,
}

/// Token type discriminator
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TokenType {
    /// Standard access token
    Access,

    /// Refresh token
    Refresh,

    /// Admin access token (for DatabaseAdminUser - legacy)
    Admin,

    /// Impersonation token
    Impersonation {
        /// The identity being impersonated
        target_identity_id: String,
        /// The admin performing the impersonation
        admin_identity_id: String,
    },
}

impl TokenType {
    /// Check if this is an access token
    pub fn is_access(&self) -> bool {
        matches!(self, TokenType::Access)
    }

    /// Check if this is a refresh token
    pub fn is_refresh(&self) -> bool {
        matches!(self, TokenType::Refresh)
    }

    /// Check if this is an impersonation token
    pub fn is_impersonation(&self) -> bool {
        matches!(self, TokenType::Impersonation { .. })
    }
}

/// Refresh token claims (minimal, server-side validation)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RefreshClaims {
    /// Subject - the identity_id
    pub sub: String,

    /// Session ID
    pub sid: String,

    /// Tenant ID
    pub tenant_id: String,

    /// Issued at time (Unix timestamp)
    pub iat: i64,

    /// Expiration time (Unix timestamp)
    pub exp: i64,

    /// JWT ID
    pub jti: String,

    /// Token family ID (for rotation detection)
    pub family: String,

    /// Generation in the family (incremented on rotation)
    pub generation: u32,

    /// User's home node path (preserved across refreshes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
}

impl RefreshClaims {
    /// Check if the token is expired
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.exp <= now
    }
}

/// Response containing auth tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokens {
    /// Access token (JWT)
    pub access_token: String,

    /// Refresh token (JWT)
    pub refresh_token: String,

    /// Token type (always "Bearer")
    pub token_type: String,

    /// Access token expiration in seconds
    pub expires_in: u64,

    /// Refresh token expiration in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_expires_in: Option<u64>,
}

impl AuthTokens {
    /// Create a new auth tokens response
    pub fn new(
        access_token: String,
        refresh_token: String,
        expires_in: u64,
        refresh_expires_in: Option<u64>,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            token_type: "Bearer".to_string(),
            expires_in,
            refresh_expires_in,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_claims_expiration() {
        let now = chrono::Utc::now().timestamp();

        let claims = AuthClaims {
            sub: "id-123".to_string(),
            email: "user@example.com".to_string(),
            tenant_id: "tenant-1".to_string(),
            repository: None,
            home: None,
            sid: "sess-123".to_string(),
            auth_strategy: "local".to_string(),
            auth_time: now,
            global_flags: GlobalFlags::default(),
            token_type: TokenType::Access,
            exp: now + 3600,
            iat: now,
            nbf: None,
            jti: "jti-123".to_string(),
            iss: None,
            aud: None,
        };

        assert!(!claims.is_expired());
        assert!(!claims.requires_reauth(300)); // 5 minutes threshold
    }

    #[test]
    fn test_sudo_mode() {
        let now = chrono::Utc::now().timestamp();

        let claims = AuthClaims {
            sub: "id-123".to_string(),
            email: "user@example.com".to_string(),
            tenant_id: "tenant-1".to_string(),
            repository: None,
            home: None,
            sid: "sess-123".to_string(),
            auth_strategy: "local".to_string(),
            auth_time: now - 600, // 10 minutes ago
            global_flags: GlobalFlags::default(),
            token_type: TokenType::Access,
            exp: now + 3600,
            iat: now,
            nbf: None,
            jti: "jti-123".to_string(),
            iss: None,
            aud: None,
        };

        // With 5-minute threshold, should require reauth
        assert!(claims.requires_reauth(300));

        // With 15-minute threshold, should not require reauth
        assert!(!claims.requires_reauth(900));
    }

    #[test]
    fn test_token_type_checks() {
        assert!(TokenType::Access.is_access());
        assert!(!TokenType::Access.is_refresh());

        assert!(TokenType::Refresh.is_refresh());
        assert!(!TokenType::Refresh.is_access());

        let impersonation = TokenType::Impersonation {
            target_identity_id: "target".to_string(),
            admin_identity_id: "admin".to_string(),
        };
        assert!(impersonation.is_impersonation());
    }
}
