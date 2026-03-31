// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Identity-based authentication HTTP handlers.
//!
//! These endpoints implement the pluggable authentication system with support
//! for multiple strategies: Local (password), Magic Link, and OIDC.
//!
//! # Endpoints
//!
//! ## Authentication
//! - `POST /auth/login` - Local identity auth (email/password)
//! - `POST /auth/magic-link` - Request magic link
//! - `GET /auth/magic-link/verify` - Verify magic link token
//! - `GET /auth/oidc/{provider}` - Start OIDC flow
//! - `GET /auth/oidc/{provider}/callback` - OIDC callback
//! - `POST /auth/refresh` - Refresh tokens
//! - `POST /auth/logout` - Revoke session
//!
//! ## Session Management
//! - `GET /auth/sessions` - List user sessions
//! - `DELETE /auth/sessions/{id}` - Revoke specific session
//!
//! ## Profile
//! - `GET /auth/me` - Get current identity
//! - `PUT /auth/me` - Update profile

// Module declarations
mod config;
mod config_types;
mod constants;
mod helpers;
mod local;
mod magic_link;
mod oidc;
mod profile;
mod session;
mod types;
mod user_node;

// Re-export all public types
pub use config_types::{
    AccessSettingsConfig, LocalAuthConfig, MagicLinkConfig, PasswordPolicyConfig,
    SessionSettingsConfig, TenantAuthConfigResponse, UpdateTenantAuthConfigRequest,
};
pub use types::{
    AuthProviderInfo, AuthProvidersResponse, AuthTokensResponse, IdentityInfo, LocalLoginRequest,
    MagicLinkRequest, MagicLinkSentResponse, MagicLinkVerifyQuery, MeForRepoResponse, MeResponse,
    OidcAuthQuery, OidcCallbackQuery, RefreshTokenRequest, RegisterRequest, SessionInfo,
    SessionsResponse,
};

// Re-export all handlers
#[cfg(feature = "storage-rocksdb")]
pub use config::{get_auth_config, update_auth_config};

#[cfg(feature = "storage-rocksdb")]
pub use local::{login, login_for_repo, register, register_for_repo};

#[cfg(feature = "storage-rocksdb")]
pub use magic_link::{request_magic_link, request_magic_link_for_repo, verify_magic_link};

#[cfg(feature = "storage-rocksdb")]
pub use oidc::{oidc_authorize, oidc_callback};

#[cfg(feature = "storage-rocksdb")]
pub use profile::{get_me, get_me_for_repo, get_providers, get_providers_for_repo};

#[cfg(feature = "storage-rocksdb")]
pub use session::{list_sessions, logout, refresh_token, revoke_session};
