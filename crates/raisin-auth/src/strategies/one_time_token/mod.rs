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

//! One-time token authentication strategy.
//!
//! This strategy implements token-based authentication for:
//!
//! - API keys (long-lived access tokens)
//! - Invite tokens (workspace/tenant invitations)
//! - Password reset tokens
//! - Email verification tokens
//! - Magic link tokens
//!
//! # Security Features
//!
//! - Tokens are generated using cryptographically secure random bytes
//! - Tokens are hashed using SHA-256 before storage (never store plaintext)
//! - Token verification uses constant-time comparison to prevent timing attacks
//! - Tokens include a prefix for easy identification and routing
//!
//! # Module Structure
//!
//! - `token_ops`: Token generation, hashing, and verification
//! - `strategy`: `AuthStrategy` trait implementation

mod strategy;
mod token_ops;

#[cfg(test)]
mod tests;

use crate::strategy::StrategyId;

/// One-time token authentication strategy.
///
/// Handles authentication via one-time use tokens such as
/// API keys, invite tokens, password reset tokens, and magic links.
///
/// The strategy itself only provides token generation and verification
/// utilities. The actual token lookup and purpose validation is handled
/// by the AuthService.
#[derive(Debug, Clone)]
pub struct OneTimeTokenStrategy {
    /// Strategy identifier
    strategy_id: StrategyId,
}

impl OneTimeTokenStrategy {
    /// Create a new one-time token authentication strategy.
    pub fn new() -> Self {
        Self {
            strategy_id: StrategyId::new(StrategyId::ONE_TIME_TOKEN),
        }
    }
}

impl Default for OneTimeTokenStrategy {
    fn default() -> Self {
        Self::new()
    }
}
