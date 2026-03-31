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

//! Authentication job definitions and helpers.
//!
//! This module provides helper functions for creating and scheduling
//! authentication-related background jobs.
//!
//! # Job Types
//!
//! - [`AuthMagicLinkSend`]: Send magic link email for passwordless authentication
//! - [`AuthSessionCleanup`]: Periodic cleanup of expired sessions
//! - [`AuthTokenCleanup`]: Periodic cleanup of expired one-time tokens
//! - [`AuthAccessNotification`]: Notify users about workspace access changes
//!
//! # Usage
//!
//! Jobs are created using the helper functions in this module and registered
//! using the unified `JobRegistry.register_job()` + `JobDataStore.put()` pattern.
//!
//! ```ignore
//! use raisin_auth::jobs::create_magic_link_job;
//!
//! // Create magic link job data
//! let (job_type, context) = create_magic_link_job(
//!     "tenant-1",
//!     "identity-123",
//!     "user@example.com",
//!     "token-456",
//! );
//!
//! // Register with job registry
//! let job_id = job_registry.register_job(
//!     job_type,
//!     Some("tenant-1".to_string()),
//!     None,
//!     None,
//!     Some(3), // max_retries
//! ).await?;
//!
//! // Store context
//! job_data_store.put(&job_id, &context)?;
//! ```

mod access_notification;
mod magic_link;
mod session_cleanup;
mod token_cleanup;

pub use access_notification::*;
pub use magic_link::*;
pub use session_cleanup::*;
pub use token_cleanup::*;

/// Notification types for access changes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessNotificationType {
    /// Access was granted to a workspace
    Granted,
    /// Access was revoked from a workspace
    Revoked,
    /// Access request was approved
    RequestApproved,
    /// Access request was denied
    RequestDenied,
    /// User was invited to a workspace
    Invited,
}

impl AccessNotificationType {
    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Revoked => "revoked",
            Self::RequestApproved => "request_approved",
            Self::RequestDenied => "request_denied",
            Self::Invited => "invited",
        }
    }

    /// Parse from string representation
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "granted" => Some(Self::Granted),
            "revoked" => Some(Self::Revoked),
            "request_approved" => Some(Self::RequestApproved),
            "request_denied" => Some(Self::RequestDenied),
            "invited" => Some(Self::Invited),
            _ => None,
        }
    }
}

impl std::fmt::Display for AccessNotificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
