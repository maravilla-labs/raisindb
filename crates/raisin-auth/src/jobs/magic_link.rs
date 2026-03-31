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

//! Magic link email job helpers.
//!
//! This module provides helpers for creating magic link email jobs.
//! The actual email sending is handled by the job handler in raisin-rocksdb.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Magic link job metadata stored in JobContext
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagicLinkJobData {
    /// The identity ID requesting the magic link
    pub identity_id: String,
    /// Email address to send the magic link to
    pub email: String,
    /// Token ID for tracking and invalidation
    pub token_id: String,
    /// The actual magic link token (stored temporarily for email template)
    pub token: String,
    /// Base URL for the magic link (e.g., "https://app.example.com")
    pub base_url: String,
    /// Expiration time in minutes (for display in email)
    pub expires_in_minutes: u32,
    /// Optional custom email template name
    pub template: Option<String>,
}

impl MagicLinkJobData {
    /// Create new magic link job data
    pub fn new(
        identity_id: impl Into<String>,
        email: impl Into<String>,
        token_id: impl Into<String>,
        token: impl Into<String>,
        base_url: impl Into<String>,
        expires_in_minutes: u32,
    ) -> Self {
        Self {
            identity_id: identity_id.into(),
            email: email.into(),
            token_id: token_id.into(),
            token: token.into(),
            base_url: base_url.into(),
            expires_in_minutes,
            template: None,
        }
    }

    /// Set custom email template
    pub fn with_template(mut self, template: impl Into<String>) -> Self {
        self.template = Some(template.into());
        self
    }

    /// Build the magic link URL
    pub fn build_link(&self) -> String {
        format!(
            "{}/auth/magic-link/verify?token={}",
            self.base_url, self.token
        )
    }

    /// Convert to metadata HashMap for JobContext
    pub fn to_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert(
            "identity_id".to_string(),
            serde_json::json!(self.identity_id),
        );
        map.insert("email".to_string(), serde_json::json!(self.email));
        map.insert("token_id".to_string(), serde_json::json!(self.token_id));
        map.insert("token".to_string(), serde_json::json!(self.token));
        map.insert("base_url".to_string(), serde_json::json!(self.base_url));
        map.insert(
            "expires_in_minutes".to_string(),
            serde_json::json!(self.expires_in_minutes),
        );
        map.insert(
            "magic_link_url".to_string(),
            serde_json::json!(self.build_link()),
        );
        if let Some(template) = &self.template {
            map.insert("template".to_string(), serde_json::json!(template));
        }
        map
    }

    /// Parse from metadata HashMap
    pub fn from_metadata(metadata: &HashMap<String, serde_json::Value>) -> Option<Self> {
        Some(Self {
            identity_id: metadata.get("identity_id")?.as_str()?.to_string(),
            email: metadata.get("email")?.as_str()?.to_string(),
            token_id: metadata.get("token_id")?.as_str()?.to_string(),
            token: metadata.get("token")?.as_str()?.to_string(),
            base_url: metadata.get("base_url")?.as_str()?.to_string(),
            expires_in_minutes: metadata.get("expires_in_minutes")?.as_u64()? as u32,
            template: metadata
                .get("template")
                .and_then(|v| v.as_str())
                .map(String::from),
        })
    }
}

/// Default email template for magic links (plain text)
pub const MAGIC_LINK_TEMPLATE_TEXT: &str = r#"
Hello,

You requested a magic link to sign in to your account.

Click here to sign in:
{magic_link_url}

This link will expire in {expires_in_minutes} minutes.

If you didn't request this link, you can safely ignore this email.

Best regards,
The Team
"#;

/// Default email template for magic links (HTML)
pub const MAGIC_LINK_TEMPLATE_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Sign In</title>
</head>
<body style="font-family: Arial, sans-serif; line-height: 1.6; color: #333;">
    <div style="max-width: 600px; margin: 0 auto; padding: 20px;">
        <h1 style="color: #2563eb;">Sign In</h1>
        <p>Hello,</p>
        <p>You requested a magic link to sign in to your account.</p>
        <p>
            <a href="{magic_link_url}"
               style="display: inline-block; background-color: #2563eb; color: white;
                      padding: 12px 24px; text-decoration: none; border-radius: 6px;
                      font-weight: bold;">
                Sign In
            </a>
        </p>
        <p style="color: #666; font-size: 14px;">
            This link will expire in {expires_in_minutes} minutes.
        </p>
        <p style="color: #999; font-size: 12px; margin-top: 30px;">
            If you didn't request this link, you can safely ignore this email.
        </p>
    </div>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_link_job_data() {
        let data = MagicLinkJobData::new(
            "identity-123",
            "user@example.com",
            "token-id-456",
            "abc123def456",
            "https://app.example.com",
            15,
        );

        assert_eq!(data.identity_id, "identity-123");
        assert_eq!(data.email, "user@example.com");
        assert_eq!(data.expires_in_minutes, 15);
        assert_eq!(
            data.build_link(),
            "https://app.example.com/auth/magic-link/verify?token=abc123def456"
        );
    }

    #[test]
    fn test_metadata_roundtrip() {
        let original = MagicLinkJobData::new(
            "identity-123",
            "user@example.com",
            "token-id-456",
            "abc123def456",
            "https://app.example.com",
            15,
        )
        .with_template("custom_template");

        let metadata = original.to_metadata();
        let restored = MagicLinkJobData::from_metadata(&metadata).unwrap();

        assert_eq!(restored.identity_id, original.identity_id);
        assert_eq!(restored.email, original.email);
        assert_eq!(restored.token, original.token);
        assert_eq!(restored.template, original.template);
    }
}
