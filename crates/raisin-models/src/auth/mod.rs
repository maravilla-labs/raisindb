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

//! Authentication and authorization models.
//!
//! This module provides models for the pluggable authentication system:
//!
//! - [`AuthContext`] - Request-scoped authentication and authorization context
//! - [`Identity`] - Global user identity within a tenant
//! - [`Session`] - Active authentication session
//! - [`AuthClaims`] - JWT claims for authenticated users
//! - [`WorkspaceAccess`] - Links identities to workspace-specific users
//! - [`TenantAuthConfig`] - Tenant-level authentication configuration
//!
//! # Architecture
//!
//! The authentication system uses a "lean JWT + hot cache" approach:
//!
//! 1. JWTs contain only identity, session ID, and global flags (< 1KB)
//! 2. Workspace-specific permissions are cached in an LRU cache
//! 3. Cache is keyed by (session_id, workspace_id) with 5-min TTL
//! 4. Cache invalidation via EventBus on role/permission changes
//!
//! # Modules
//!
//! - `context` - Request-scoped AuthContext
//! - `identity` - Identity, LinkedProvider, LocalCredentials
//! - `session` - Session, OneTimeToken, TokenPurpose
//! - `claims` - AuthClaims, GlobalFlags, TokenType
//! - `access` - WorkspaceAccess, AccessStatus, AccessSettings
//! - `config` - TenantAuthConfig, AuthProviderConfig, PasswordPolicy

mod access;
mod claims;
mod config;
mod context;
mod identity;
mod session;

pub use access::*;
pub use claims::*;
pub use config::*;
pub use context::*;
pub use identity::*;
pub use session::*;
