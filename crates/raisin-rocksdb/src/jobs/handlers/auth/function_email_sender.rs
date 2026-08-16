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

//! Magic-link delivery through a tenant-owned FUNCTION.
//!
//! # Why a function and not a provider call from Rust
//!
//! The email itself — subject line, wording, branding, which address it comes
//! from — is tenant content, and this crate has no business holding it. The
//! function at [`SEND_MAGIC_LINK_PATH`] renders the templates and calls
//! `raisin.email.send`, which means the send passes through the two gates that
//! already exist for functions: the function's `email_policy` (deny-by-default
//! recipient allow-list) and its `secret_policy` (which is what lets it read
//! `email/api_key` at all). A hardcoded Rust sender would bypass both, and the
//! tenant could neither restyle the mail nor restrict who it may reach.
//!
//! The invocation is identical in shape to
//! [`crate::jobs::handlers::virtual_mount_sync::FunctionAdapterInvoker`]: same
//! callback, same `functions` workspace, same system context (the function is
//! operator-installed, and the caller — someone who has not logged in yet — has
//! no principal that could pass RLS).

use async_trait::async_trait;
use raisin_auth::jobs::MagicLinkJobData;
use raisin_error::{Error, Result};
use serde_json::json;

use super::magic_link::{MagicLinkEmailSender, MagicLinkScope};
use crate::jobs::handlers::FunctionExecutorCallback;

/// Workspace functions live in.
const FUNCTIONS_WORKSPACE: &str = "functions";

/// Path of the function that renders and sends the magic-link email.
///
/// Under `/lib/raisin/auth/` with the rest of the built-in auth functions:
/// that prefix is the sync-filter root the `raisin-auth` builtin package owns,
/// so the function is installed, updated and conflict-resolved with its
/// siblings rather than sitting outside any package's remit.
pub const SEND_MAGIC_LINK_PATH: &str = "/lib/raisin/auth/send-magic-link";

/// [`MagicLinkEmailSender`] that delegates to [`SEND_MAGIC_LINK_PATH`].
pub struct FunctionMagicLinkEmailSender {
    executor: FunctionExecutorCallback,
}

impl FunctionMagicLinkEmailSender {
    /// Wrap a function executor callback.
    pub fn new(executor: FunctionExecutorCallback) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl MagicLinkEmailSender for FunctionMagicLinkEmailSender {
    async fn send_magic_link(&self, scope: &MagicLinkScope, data: &MagicLinkJobData) -> Result<()> {
        // The link is built here rather than in JS so the URL the user clicks
        // is assembled from the same `base_url` that the request handler
        // allow-listed against — see the module docs on `MagicLinkJobData`.
        let input = json!({
            "email": data.email,
            "magic_link_url": data.build_link(),
            "expires_in_minutes": data.expires_in_minutes,
            "template": data.template,
        });

        let execution_id = format!("magic-link-{}", nanoid::nanoid!());
        let result = (self.executor)(
            SEND_MAGIC_LINK_PATH.to_string(),
            execution_id,
            input,
            scope.tenant_id.clone(),
            scope.repo_id.clone(),
            scope.branch.clone(),
            FUNCTIONS_WORKSPACE.to_string(),
            None, // system context: the requester is not authenticated yet
            None, // no live log streaming for a background send
        )
        .await
        .map_err(|e| Error::internal(format!("magic link send function failed: {e}")))?;

        if result.success {
            return Ok(());
        }

        // Deliberately an error, not a warning: the job's whole purpose is the
        // email. Returning Ok here would retire the job as done and leave the
        // user waiting for a link that was never sent.
        Err(Error::internal(format!(
            "magic link send function at {SEND_MAGIC_LINK_PATH} reported failure: {}",
            result
                .error
                .unwrap_or_else(|| "no message given".to_string())
        )))
    }
}
