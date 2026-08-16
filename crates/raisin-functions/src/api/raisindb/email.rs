// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Transactional email sending for RaisinFunctionApi.
//!
//! The sender identity is NOT something a function chooses: it comes from the
//! tenant's `raisin:EmailConfig` node at `/config/email` in the `raisin:system`
//! workspace, exactly as `raisin:RepoAuthConfig` supplies the auth identity.
//! A function supplies only recipients, subject and bodies — so a compromised
//! function cannot send as an address the tenant never verified.
//!
//! Like `imap.rs`, everything that can refuse the call runs BEFORE the socket,
//! cheapest and most restrictive first: every recipient must be allowed by the
//! function's `email_policy` (which denies by default), the config must exist
//! and be explicitly enabled, and the credential must be readable under the
//! function's `secret_policy`. Only then does the provider module open a
//! connection. The resolved credential is passed straight into
//! [`EmailProvider::send`] and never stored, logged, or returned.

use raisin_error::Result;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::runtime::email::{EmailConfig, EmailError, EmailMessage};

/// Workspace holding per-tenant operator configuration.
const CONFIG_WORKSPACE: &str = "raisin:system";
/// Path of the tenant's `raisin:EmailConfig` node.
const EMAIL_CONFIG_PATH: &str = "/config/email";

impl RaisinFunctionApi {
    /// The policy gate for one send's recipient list.
    ///
    /// Runs FIRST — before the config node is read and before the credential is
    /// resolved, let alone before a socket is opened — so a function that was
    /// never granted email learns that from the policy, not from a provider
    /// error, and a denied send never causes the credential to be decrypted.
    ///
    /// Every recipient must be allowed. There is no partial send: a message
    /// addressed to one permitted and one forbidden domain is refused whole,
    /// because silently dropping a recipient would let a caller believe a
    /// message went somewhere it did not.
    fn authorize_recipients(&self, to: &[String]) -> Result<()> {
        // Deliberately no subject or body in any of these messages — only the
        // recipient, which the caller already supplied.
        let denied: Vec<&String> = to
            .iter()
            .filter(|addr| !self.email_policy.is_recipient_allowed(addr))
            .collect();

        if denied.is_empty() {
            return Ok(());
        }

        let detail = if !self.email_policy.enabled {
            "this function has no email_policy (sending is denied by default)".to_string()
        } else if self.email_policy.allowed_recipients.is_empty() {
            "this function's email_policy has an empty allowed_recipients list".to_string()
        } else {
            format!(
                "not matched by this function's email_policy.allowed_recipients {:?}",
                self.email_policy.allowed_recipients
            )
        };

        tracing::warn!(
            denied_recipients = denied.len(),
            total_recipients = to.len(),
            policy_enabled = self.email_policy.enabled,
            "raisin.email.send denied by email policy"
        );

        Err(raisin_error::Error::PermissionDenied(format!(
            "[email:policy_denied] cannot send to {denied:?}: {detail}. Grant it by adding \
             the recipient domain to the function's email_policy.allowed_recipients."
        )))
    }

    /// Load `/config/email`, check it permits sending, and resolve the
    /// credential it references — all before any provider is contacted.
    ///
    /// The credential is returned rather than kept, so it lives only as long as
    /// the one send it is handed to.
    async fn authorize_email(&self) -> Result<(EmailConfig, String)> {
        let node = self
            .impl_node_get(CONFIG_WORKSPACE, EMAIL_CONFIG_PATH)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::InvalidState(format!(
                    "[email:config] outbound email is not configured for this tenant: \
                     no raisin:EmailConfig node at {CONFIG_WORKSPACE}{EMAIL_CONFIG_PATH}"
                ))
            })?;

        let properties = node.get("properties").ok_or_else(|| {
            raisin_error::Error::InvalidState(format!(
                "[email:config] {EMAIL_CONFIG_PATH} has no properties"
            ))
        })?;

        // The nodetype leaves `enabled` optional with a `false` default, and an
        // absent property must read as "not enabled": a tenant that provisioned
        // the node but has not finished verifying its sending domain would
        // otherwise start sending the moment the node appeared.
        if properties.get("enabled").and_then(Value::as_bool) != Some(true) {
            return Err(raisin_error::Error::InvalidState(
                "[email:config] outbound email is not enabled for this tenant \
                 (set enabled: true on /config/email once the sending domain is verified)"
                    .to_string(),
            ));
        }

        let config = EmailConfig::from_value(properties).map_err(raisin_error::Error::from)?;

        // Resolving the reference runs the function's secret policy, so a
        // function that was never granted the email credential is refused here
        // — with the policy's own message, naming what would grant it.
        let credential = self.impl_secret_resolve(&config.credential_ref).await?;

        Ok((config, credential))
    }

    /// Back `raisin.email.send(message)`.
    ///
    /// `message` is `{ to, subject, text, html? }`; `to` accepts either a single
    /// address or an array. Returns the provider's receipt
    /// `{ message_id, provider }` — proof of acceptance, not of delivery.
    pub(crate) async fn impl_email_send(&self, message: Value) -> Result<Value> {
        let message = parse_message(message)?;
        self.authorize_recipients(&message.to)?;
        let (config, credential) = self.authorize_email().await?;

        let receipt = config
            .provider
            .send(&config, &credential, &message)
            .await
            .map_err(raisin_error::Error::from)?;

        tracing::info!(
            provider = config.provider.as_str(),
            message_id = %receipt.message_id,
            recipients = message.to.len(),
            "email sent"
        );

        serde_json::to_value(receipt)
            .map_err(|e| raisin_error::Error::Internal(format!("email serialize failed: {e}")))
    }
}

/// Parse the caller's message value into an [`EmailMessage`].
///
/// `to` is normalized from the single-recipient spelling (`"a@b.c"`) that both
/// runtimes' authors reach for first; everything else must already match the
/// wire shape. Field-level validation (header splitting, empty bodies,
/// recipient count) stays in `EmailMessage::validate`, which the provider runs
/// before opening a socket.
fn parse_message(mut message: Value) -> Result<EmailMessage> {
    if let Some(to) = message.get_mut("to") {
        if let Some(single) = to.as_str() {
            *to = Value::Array(vec![Value::String(single.to_string())]);
        }
    }
    serde_json::from_value(message).map_err(|e| {
        raisin_error::Error::from(EmailError::InvalidMessage(format!(
            "message must be {{ to, subject, text, html? }}: {e}"
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_recipient_string_is_accepted() {
        let parsed = parse_message(serde_json::json!({
            "to": "user@example.com",
            "subject": "Sign in",
            "text": "link",
        }))
        .expect("single recipient");
        assert_eq!(parsed.to, vec!["user@example.com".to_string()]);
        assert!(parsed.html.is_none());
    }

    #[test]
    fn a_message_without_a_text_body_is_refused_before_anything_else() {
        // Not a panic and not a provider round-trip: the caller gets a
        // validation error naming the shape it should have sent.
        let err = parse_message(serde_json::json!({
            "to": ["user@example.com"],
            "subject": "Sign in",
        }))
        .expect_err("text is required");
        assert!(
            err.to_string().contains("email:invalid_message"),
            "unexpected error: {err}"
        );
    }
}
