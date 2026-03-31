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

//! Access notification email handler

use async_trait::async_trait;
use raisin_auth::jobs::AccessNotificationJobData;
use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use std::sync::Arc;

/// Callback trait for sending access notification emails
#[async_trait]
pub trait AccessNotificationEmailSender: Send + Sync {
    /// Send an access notification email
    ///
    /// # Arguments
    ///
    /// * `data` - Access notification job data
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on success, or an error if sending fails.
    async fn send_access_notification(&self, data: &AccessNotificationJobData) -> Result<()>;
}

/// Handler for access notification jobs
pub struct AuthAccessNotificationHandler<S: AccessNotificationEmailSender> {
    email_sender: Arc<S>,
}

impl<S: AccessNotificationEmailSender> AuthAccessNotificationHandler<S> {
    /// Create a new access notification handler
    pub fn new(email_sender: Arc<S>) -> Self {
        Self { email_sender }
    }

    /// Handle access notification job
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<()> {
        // Verify job type
        match &job.job_type {
            JobType::AuthAccessNotification { .. } => {}
            _ => {
                return Err(Error::Validation(
                    "Expected AuthAccessNotification job type".to_string(),
                ))
            }
        };

        // Parse job data from context metadata
        let data =
            AccessNotificationJobData::from_metadata(&context.metadata).ok_or_else(|| {
                Error::Validation(
                    "Invalid or missing access notification job data in context".to_string(),
                )
            })?;

        tracing::info!(
            job_id = %job.id,
            email = %data.email,
            identity_id = %data.identity_id,
            repo_id = %data.repo_id,
            notification_type = %data.notification_type,
            "Sending access notification email"
        );

        // Send the email
        self.email_sender.send_access_notification(&data).await?;

        tracing::info!(
            job_id = %job.id,
            email = %data.email,
            notification_type = %data.notification_type,
            "Access notification email sent successfully"
        );

        Ok(())
    }
}
