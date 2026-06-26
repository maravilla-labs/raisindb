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
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
//! │  │   Local     │  │   OIDC      │  │  Magic Link │  ...    │
//! │  │  Strategy   │  │  Strategy   │  │  Strategy   │         │
//! │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘         │
//! │         └────────────────┼────────────────┘                 │
//! │                          ▼                                  │
//! │              ┌───────────────────────┐                      │
//! │              │   AuthStrategyRegistry │                     │
//! │              └───────────┬───────────┘                      │
//! │                          ▼                                  │
//! │              ┌───────────────────────┐                      │
//! │              │      AuthService      │                      │
//! │              └───────────────────────┘                      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use raisin_auth::{AuthStrategyRegistry, AuthService, strategies::LocalStrategy};
//!
//! // Create registry and register strategies
//! let registry = AuthStrategyRegistry::new();
//! registry.register(Arc::new(LocalStrategy::new())).await;
//!
//! // Create auth service
//! let auth_service = AuthService::new(
//!     registry,
//!     identity_store,
//!     session_store,
//!     jwt_secret,
//! );
//!
//! // Authenticate
//! let tokens = auth_service.authenticate(
//!     "tenant-1",
//!     AuthCredentials::UsernamePassword {
//!         username: "user@example.com".to_string(),
//!         password: "password".to_string(),
//!     },
//! ).await?;
//! ```
//!
//! # Features
//!
//! - `oidc`: Enable OIDC support (requires `reqwest`)

pub mod authserver;
pub mod cache;
pub mod jobs;
pub mod registry;
pub mod strategies;
pub mod strategy;

// Re-export main types
pub use registry::AuthStrategyRegistry;
pub use strategy::{AuthCredentials, AuthStrategy, AuthenticationResult, StrategyId};

// Re-export models for convenience
pub use raisin_models::auth::{
    AccessSettings, AccessStatus, AuthClaims, AuthProviderConfig, AuthTokens, GlobalFlags,
    Identity, LinkedProvider, LocalCredentials, OneTimeToken, PasswordPolicy, RefreshClaims,
    Session, SessionSettings, TenantAuthConfig, TokenPurpose, TokenType, WorkspaceAccess,
};
