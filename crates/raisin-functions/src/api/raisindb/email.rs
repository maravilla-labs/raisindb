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
//! A function MAY choose WHICH of the tenant's configured providers to send
//! through, by name (`{ provider: "marketing" }`). That is a choice between
//! identities the operator already set up, not a new one: an unknown name is an
//! error rather than a fall-back to the default, so a typo cannot silently
//! move mail onto another account. Omitting it uses the tenant's default, which
//! is what every system-originated mail (magic link above all) does.
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
use crate::runtime::email::{EmailConfig, EmailError, EmailMessage, EmailProvider, EmailSender};

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

    /// Read and parse `/config/email`.
    ///
    /// Split from selection because two callers need it: a send (which then
    /// resolves one sender) and `providers()` (which lists them all).
    async fn email_config(&self) -> Result<EmailConfig> {
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

        EmailConfig::from_value(properties).map_err(raisin_error::Error::from)
    }

    /// Load `/config/email`, pick the sender for this send, and resolve the
    /// credential it references — all before any provider is contacted.
    ///
    /// The credential is returned rather than kept, so it lives only as long as
    /// the one send it is handed to. Note that `enabled` and provider selection
    /// are both decided by [`EmailConfig::resolve`] rather than re-checked
    /// here: the console, the send path and any future caller must agree about
    /// which account is the default, and they can only do that by asking the
    /// same function.
    async fn authorize_email(&self, requested: Option<&str>) -> Result<(EmailSender, String)> {
        let config = self.email_config().await?;
        let sender = config
            .resolve(requested)
            .map_err(raisin_error::Error::from)?;

        // Resolving the reference runs the function's secret policy, so a
        // function that was never granted the email credential is refused here
        // — with the policy's own message, naming what would grant it.
        let credential = self.impl_secret_resolve(&sender.credential_ref).await?;

        Ok((sender, credential))
    }

    /// Back `raisin.email.send(message)`.
    ///
    /// `message` is `{ to, subject, text, html?, provider? }`; `to` accepts
    /// either a single address or an array. Returns the receipt
    /// `{ message_id, provider, sender }` — proof of acceptance, not of
    /// delivery.
    pub(crate) async fn impl_email_send(&self, message: Value) -> Result<Value> {
        let (message, requested_provider) = parse_message(message)?;
        self.authorize_recipients(&message.to)?;
        let (sender, credential) = self.authorize_email(requested_provider.as_deref()).await?;

        let receipt = EmailProvider::send(&sender, &credential, &message)
            .await
            .map_err(raisin_error::Error::from)?;

        tracing::info!(
            provider = sender.provider.as_str(),
            sender = %sender.name,
            message_id = %receipt.message_id,
            recipients = message.to.len(),
            "email sent"
        );

        serde_json::to_value(receipt)
            .map_err(|e| raisin_error::Error::Internal(format!("email serialize failed: {e}")))
    }

    /// Back `raisin.email.providers()`.
    ///
    /// Lists what a function may name, so a function does not have to hardcode
    /// a provider name it cannot verify exists. Deliberately carries no
    /// credential and no `credential_ref`: knowing WHICH secret backs a sender
    /// is operator information, and a function that may send does not thereby
    /// get to enumerate the secret store.
    pub(crate) async fn impl_email_providers(&self) -> Result<Value> {
        // Gated on the same policy as sending, and for the same reason: only a
        // function that may send has any use for the list. A function without
        // `email_policy` learns that here rather than discovering the sender
        // names of a tenant it can never send as.
        if !self.email_policy.enabled {
            return Err(raisin_error::Error::PermissionDenied(
                "[email:policy_denied] this function has no email_policy \
                 (email is denied by default). Grant it by declaring \
                 email_policy in the function's .node.yaml."
                    .to_string(),
            ));
        }

        let config = self.email_config().await?;
        let listed: Vec<Value> = config
            .senders()
            .into_iter()
            .map(|(sender, enabled, is_default)| {
                serde_json::json!({
                    "name": sender.name,
                    "provider": sender.provider.as_str(),
                    "from_address": sender.from_address,
                    "enabled": enabled,
                    "default": is_default,
                })
            })
            .collect();

        Ok(serde_json::json!({
            // The tenant master switch, so a caller can tell "nothing
            // configured" from "configured but switched off" — the two look
            // identical from a failed send.
            "enabled": config.enabled,
            "providers": listed,
        }))
    }
}

/// Parse the caller's message value into an [`EmailMessage`] plus the provider
/// it named, if any.
///
/// `to` is normalized from the single-recipient spelling (`"a@b.c"`) that both
/// runtimes' authors reach for first; everything else must already match the
/// wire shape. Field-level validation (header splitting, empty bodies,
/// recipient count) stays in `EmailMessage::validate`, which the provider runs
/// before opening a socket.
///
/// `provider` is REMOVED from the value before it is deserialized, not left for
/// serde to ignore. An unknown-field error is not available here (the message
/// type is deliberately permissive), so leaving it in would mean a misspelled
/// key silently became part of nothing.
fn parse_message(mut message: Value) -> Result<(EmailMessage, Option<String>)> {
    let requested_provider = message
        .as_object_mut()
        .and_then(|obj| obj.remove("provider"))
        .and_then(|v| match v {
            Value::String(s) => Some(s),
            // `null` is what an unset variable serializes to in both runtimes;
            // it means "the default", not "a provider named null".
            Value::Null => None,
            _ => Some(String::new()),
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(to) = message.get_mut("to") {
        if let Some(single) = to.as_str() {
            *to = Value::Array(vec![Value::String(single.to_string())]);
        }
    }
    let parsed: EmailMessage = serde_json::from_value(message).map_err(|e| {
        raisin_error::Error::from(EmailError::InvalidMessage(format!(
            "message must be {{ to, subject, text, html?, provider? }}: {e}"
        )))
    })?;
    Ok((parsed, requested_provider))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_recipient_string_is_accepted() {
        let (parsed, provider) = parse_message(serde_json::json!({
            "to": "user@example.com",
            "subject": "Sign in",
            "text": "link",
        }))
        .expect("single recipient");
        assert_eq!(parsed.to, vec!["user@example.com".to_string()]);
        assert!(parsed.html.is_none());
        assert!(provider.is_none(), "no provider named means the default");
    }

    #[test]
    fn a_named_provider_is_lifted_out_of_the_message() {
        let (parsed, provider) = parse_message(serde_json::json!({
            "to": "user@example.com",
            "subject": "Sign in",
            "text": "link",
            "provider": "  marketing  ",
        }))
        .expect("named provider");
        assert_eq!(provider.as_deref(), Some("marketing"));
        // And it did not become part of the message itself.
        assert_eq!(parsed.subject, "Sign in");
    }

    /// An unset template variable serializes to `null` or `""` in both
    /// runtimes; both must read as "use the default" rather than as a provider
    /// name that cannot exist.
    #[test]
    fn an_absent_provider_value_means_the_default() {
        for value in [serde_json::Value::Null, serde_json::json!("   ")] {
            let (_, provider) = parse_message(serde_json::json!({
                "to": "user@example.com",
                "subject": "Sign in",
                "text": "link",
                "provider": value,
            }))
            .expect("parses");
            assert!(provider.is_none());
        }
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
