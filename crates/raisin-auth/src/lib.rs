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

//! Pluggable authentication system for RaisinDB.
//!
//! This crate provides a passport.js-style authentication framework with:
//!
//! - **Pluggable strategies**: Local, Magic Link, OIDC (Google, Okta, Keycloak, Azure AD)
//! - **Lean JWT + Hot Cache**: Small tokens with cached workspace permissions
//! - **Session management**: Server-side sessions with refresh token rotation
//! - **Identity linking**: Multiple auth providers per user
//! - **Workspace access control**: Request/invite mechanisms
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     AUTHENTICATION LAYER                    │
//! │                                                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
//! │  │   Local     │  │   OIDC      │  │  Magic Link │  ...     │
//! │  │  Strategy   │  │  Strategy   │  │  Strategy   │          │
//! │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘          │
//! │         └────────────────┼────────────────┘                 │
//! │                          ▼                                  │
//! │              ┌───────────────────────┐                      │
//! │              │  transport handlers   │                      │
//! │              │  + AuthService (JWT)  │                      │
//! │              └───────────────────────┘                      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # How a strategy is reached
//!
//! There is no registry. A transport handler loads the tenant's
//! [`raisin_models::auth::TenantAuthConfig`], picks the provider the request
//! names, builds the strategy for it and uses it once. A strategy therefore
//! always sees the configuration as it stands right now, which is what lets an
//! administrator change a provider without restarting the server, and there is
//! exactly one place that decides which strategy runs.
//!
//! An earlier `AuthStrategyRegistry` held strategies initialised at startup and
//! was never wired to a transport. It is gone rather than left beside the
//! working path, because two mechanisms for the same decision drift.
//!
//! # OIDC login
//!
//! ```ignore
//! use raisin_auth::strategies::{OidcStrategy, OidcLoginState};
//! use raisin_auth::AuthStrategy;
//!
//! let mut strategy = OidcStrategy::new("google", "Sign in with Google");
//! strategy.init(&provider_config, Some(&client_secret)).await?;
//!
//! // Send the browser here. The sealed state carries the PKCE verifier.
//! let redirect = strategy.begin_login(tenant_id, &master_key, app_redirect, repo)?;
//!
//! // ... the provider redirects back with `code` and `state` ...
//! let state = OidcLoginState::open(&state_param, &master_key, tenant_id, "google")?;
//! let result = strategy.complete_login(&state, &code).await?;
//! ```
//!
pub mod authserver;
pub mod cache;
pub mod jobs;
pub mod strategies;
pub mod strategy;

// Re-export main types
pub use strategy::{AuthCredentials, AuthStrategy, AuthenticationResult, StrategyId};

// Re-export models for convenience
pub use raisin_models::auth::{
    AccessSettings, AccessStatus, AuthClaims, AuthProviderConfig, AuthTokens, GlobalFlags,
    Identity, LinkedProvider, LocalCredentials, OneTimeToken, PasswordPolicy, RefreshClaims,
    Session, SessionSettings, TenantAuthConfig, TokenPurpose, TokenType, WorkspaceAccess,
};
