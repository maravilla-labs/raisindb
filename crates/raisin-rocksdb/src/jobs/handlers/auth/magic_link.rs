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

//! Magic link email handler

use async_trait::async_trait;
use raisin_auth::jobs::MagicLinkJobData;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Where a magic-link send runs.
///
/// The job data says WHAT to send; this says WHOSE. Both halves are needed
/// because delivery goes through a tenant-owned function, and a function is
/// addressed by tenant + repo + branch — none of which can be derived from an
/// email address and a token.
#[derive(Debug, Clone)]
pub struct MagicLinkScope {
    /// Tenant the magic link was requested in.
    pub tenant_id: String,
    /// Repository the magic link was requested for.
    pub repo_id: String,
    /// Branch the sending function is resolved on.
    pub branch: String,
}

/// Callback trait for sending magic link emails
#[async_trait]
pub trait MagicLinkEmailSender: Send + Sync {
    /// Send a magic link email
    ///
    /// # Arguments
    ///
    /// * `scope` - Tenant/repo/branch the send runs in
    /// * `data` - Magic link job data containing email, token, etc.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success, or an error if sending fails.
    async fn send_magic_link(&self, scope: &MagicLinkScope, data: &MagicLinkJobData) -> Result<()>;
}

/// Handler for sending magic link emails
pub struct AuthMagicLinkSendHandler<S: MagicLinkEmailSender> {
    email_sender: Arc<S>,
}

impl<S: MagicLinkEmailSender> AuthMagicLinkSendHandler<S> {
    /// Create a new magic link send handler
    pub fn new(email_sender: Arc<S>) -> Self {
        Self { email_sender }
    }

    /// Handle magic link send job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Verify job type
        match &job.job_type {
            JobType::AuthMagicLinkSend { .. } => {}
            _ => {
                return Err(Error::Validation(
                    "Expected AuthMagicLinkSend job type".to_string(),
                ))
            }
        };

        // Parse job data from context metadata
        let data = MagicLinkJobData::from_metadata(&context.metadata).ok_or_else(|| {
            Error::Validation("Invalid or missing magic link job data in context".to_string())
        })?;

        tracing::info!(
            job_id = %job.id,
            email = %data.email,
            identity_id = %data.identity_id,
            "Sending magic link email"
        );

        // The scope travels on the job context, not in the job type: the
        // sender needs a repo and a branch to resolve the sending function,
        // and JobType::AuthMagicLinkSend carries only identity/email/token.
        let scope = MagicLinkScope {
            tenant_id: context.tenant_id.clone(),
            repo_id: context.repo_id.clone(),
            branch: context.branch.clone(),
        };

        // Send the email
        self.email_sender.send_magic_link(&scope, &data).await?;

        tracing::info!(
            job_id = %job.id,
            email = %data.email,
            "Magic link email sent successfully"
        );

        Ok(())
    }
}
