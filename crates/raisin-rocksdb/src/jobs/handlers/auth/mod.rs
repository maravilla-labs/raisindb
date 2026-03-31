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

// TODO(v0.2): Public API for job handler injection via JobRegistry
#![allow(dead_code)]

//! Authentication job handlers.
//!
//! This module provides handlers for authentication-related background jobs:
//!
//! - [`AuthMagicLinkSendHandler`]: Send magic link emails
//! - [`AuthSessionCleanupHandler`]: Clean up expired sessions
//! - [`AuthTokenCleanupHandler`]: Clean up expired one-time tokens
//! - [`AuthAccessNotificationHandler`]: Notify users about access changes
//! - [`AuthCreateUserNodeHandler`]: Create user nodes on registration
//!
//! # Architecture
//!
//! These handlers use callback traits to delegate actual operations (email sending,
//! session/token deletion) to implementations that are injected at runtime. This
//! allows for:
//!
//! - Testability (mock implementations)
//! - Flexibility (different email providers, storage backends)
//! - Separation of concerns (handler logic vs. infrastructure)

mod access_notification;
mod magic_link;
mod rocksdb_user_node;
mod session_cleanup;
mod token_cleanup;
mod user_node;

#[cfg(test)]
mod tests;

pub use self::access_notification::{AccessNotificationEmailSender, AuthAccessNotificationHandler};
pub use self::magic_link::{AuthMagicLinkSendHandler, MagicLinkEmailSender};
pub use self::rocksdb_user_node::RocksDBUserNodeCreator;
pub use self::session_cleanup::{AuthSessionCleanupHandler, SessionCleanupStore};
pub use self::token_cleanup::{AuthTokenCleanupHandler, TokenCleanupStore};
pub use self::user_node::{
    AuthCreateUserNodeHandler, CreateUserNodeJobData, CreateUserNodeResult, UserNodeCreator,
};

/// Convert email to a safe node name
pub(super) fn email_to_node_name(email: &str) -> String {
    email
        .to_lowercase()
        .replace('@', "-at-")
        .replace('.', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}
