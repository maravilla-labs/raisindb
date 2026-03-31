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

/// Callback trait for sending magic link emails
#[async_trait]
pub trait MagicLinkEmailSender: Send + Sync {
    /// Send a magic link email
    ///
    /// # Arguments
    ///
    /// * `data` - Magic link job data containing email, token, etc.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success, or an error if sending fails.
    async fn send_magic_link(&self, data: &MagicLinkJobData) -> Result<()>;
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

        // Send the email
        self.email_sender.send_magic_link(&data).await?;

        tracing::info!(
            job_id = %job.id,
            email = %data.email,
            "Magic link email sent successfully"
        );

        Ok(())
    }
}
