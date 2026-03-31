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

//! Authentication strategy implementations.
//!
//! This module provides concrete implementations of the `AuthStrategy` trait
//! for different authentication methods.
//!
//! # Available Strategies
//!
//! - [`LocalStrategy`] - Username/password authentication with bcrypt
//! - [`MagicLinkStrategy`] - Passwordless email authentication
//! - [`OneTimeTokenStrategy`] - API keys, invite tokens, and one-time tokens
//! - [`OidcStrategy`] - OpenID Connect (Google, Okta, Azure AD, etc.)
//!
//! # Future Strategies
//!
//! - `SamlStrategy` - SAML 2.0 authentication

mod local;
mod magic_link;
mod oidc;
mod one_time_token;

pub use local::LocalStrategy;
pub use magic_link::MagicLinkStrategy;
pub use oidc::OidcStrategy;
pub use one_time_token::OneTimeTokenStrategy;
