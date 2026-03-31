// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Request and response types for identity authentication.

use serde::{Deserialize, Serialize};

#[cfg(feature = "storage-rocksdb")]
use raisin_models::auth::Identity;

// ============================================================================
// Request Types
// ============================================================================

/// Request for user registration
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// Email address
    pub email: String,
    /// Password
    pub password: String,
    /// Optional display name
    pub display_name: Option<String>,
}

/// Request for local (email/password) authentication
#[derive(Debug, Deserialize)]
pub struct LocalLoginRequest {
    /// Email address
    pub email: String,
    /// Password
    pub password: String,
    /// Remember me (longer session)
    #[serde(default)]
    pub remember_me: bool,
}

/// Request for magic link authentication
#[derive(Debug, Deserialize)]
pub struct MagicLinkRequest {
    /// Email address to send the magic link to
    pub email: String,
    /// Optional redirect URL after authentication
    pub redirect_url: Option<String>,
}

/// Query parameters for magic link verification
#[derive(Debug, Deserialize)]
pub struct MagicLinkVerifyQuery {
    /// The magic link token
    pub token: String,
}

/// Request to refresh authentication tokens
#[derive(Debug, Deserialize)]
pub struct RefreshTokenRequest {
    /// The refresh token
    pub refresh_token: String,
}

/// Query parameters for OIDC authorization
#[derive(Debug, Deserialize)]
pub struct OidcAuthQuery {
    /// Optional redirect URL after authentication
    pub redirect_url: Option<String>,
}

/// Query parameters for OIDC callback
#[derive(Debug, Deserialize)]
pub struct OidcCallbackQuery {
    /// Authorization code
    pub code: String,
    /// State parameter
    pub state: String,
    /// Error code (if authorization failed)
    pub error: Option<String>,
    /// Error description
    pub error_description: Option<String>,
}

// ============================================================================
// Response Types
// ============================================================================

/// Authentication tokens response
#[derive(Debug, Serialize)]
pub struct AuthTokensResponse {
    /// Access token (JWT)
    pub access_token: String,
    /// Refresh token
    pub refresh_token: String,
    /// Token type (always "Bearer")
    pub token_type: String,
    /// Access token expiration time (Unix timestamp)
    pub expires_at: i64,
    /// Identity information
    pub identity: IdentityInfo,
}

/// Identity information in auth responses
#[derive(Debug, Serialize)]
pub struct IdentityInfo {
    /// Identity ID
    pub id: String,
    /// Email address
    pub email: String,
    /// Display name
    pub display_name: Option<String>,
    /// Avatar URL
    pub avatar_url: Option<String>,
    /// Whether email is verified
    pub email_verified: bool,
    /// Linked authentication providers
    pub linked_providers: Vec<String>,
    /// User's home node path in the repository
    pub home: Option<String>,
}

#[cfg(feature = "storage-rocksdb")]
impl IdentityInfo {
    /// Create IdentityInfo from an Identity and optional home path.
    pub fn from_identity(identity: &Identity, home: Option<String>) -> Self {
        Self {
            id: identity.identity_id.clone(),
            email: identity.email.clone(),
            display_name: identity.display_name.clone(),
            avatar_url: identity.avatar_url.clone(),
            email_verified: identity.email_verified,
            linked_providers: identity
                .linked_providers
                .iter()
                .map(|p| p.strategy_id.clone())
                .collect(),
            home,
        }
    }
}

/// Magic link sent response
#[derive(Debug, Serialize)]
pub struct MagicLinkSentResponse {
    /// Confirmation message
    pub message: String,
    /// Masked email for display (e.g., "u***@example.com")
    pub masked_email: String,
    /// Expiration time in minutes
    pub expires_in_minutes: u32,
}

/// Session information
#[derive(Debug, Serialize)]
pub struct SessionInfo {
    /// Session ID
    pub id: String,
    /// Authentication strategy used
    pub auth_strategy: String,
    /// Device/user agent info
    pub user_agent: Option<String>,
    /// IP address
    pub ip_address: Option<String>,
    /// Created at (ISO 8601)
    pub created_at: String,
    /// Last active (ISO 8601)
    pub last_active_at: String,
    /// Whether this is the current session
    pub is_current: bool,
}

/// List of sessions response
#[derive(Debug, Serialize)]
pub struct SessionsResponse {
    /// List of sessions
    pub sessions: Vec<SessionInfo>,
}

/// Available auth providers for a tenant
#[derive(Debug, Serialize)]
pub struct AuthProvidersResponse {
    /// Available authentication providers
    pub providers: Vec<AuthProviderInfo>,
    /// Whether local (password) authentication is enabled
    pub local_enabled: bool,
    /// Whether magic link authentication is enabled
    pub magic_link_enabled: bool,
}

/// Information about an auth provider
#[derive(Debug, Serialize)]
pub struct AuthProviderInfo {
    /// Provider ID (e.g., "google", "okta")
    pub id: String,
    /// Display name (e.g., "Sign in with Google")
    pub display_name: String,
    /// Icon identifier
    pub icon: String,
    /// Authorization URL
    pub auth_url: String,
}

/// Response for /auth/me endpoint
#[derive(Debug, Serialize)]
pub struct MeResponse {
    /// Identity ID (global identity_id from JWT)
    pub id: String,
    /// Email address
    pub email: Option<String>,
    /// Effective roles
    pub roles: Vec<String>,
    /// Groups the user belongs to
    pub groups: Vec<String>,
    /// Whether this is an anonymous user
    pub anonymous: bool,
    /// User's home path (raisin:User node path in raisin:access_control workspace)
    pub home: Option<String>,
}

/// Response for /auth/{repo}/me endpoint
#[derive(Debug, Serialize)]
pub struct MeForRepoResponse {
    /// Identity ID (global identity_id from JWT)
    pub id: String,
    /// Email address
    pub email: Option<String>,
    /// Effective roles
    pub roles: Vec<String>,
    /// Whether this is an anonymous user
    pub anonymous: bool,
    /// User's home path (raisin:User node path in raisin:access_control workspace)
    pub home: Option<String>,
    /// User's node from the repository (if exists)
    pub user_node: Option<raisin_models::nodes::Node>,
}
