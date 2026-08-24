// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Native transactional-email binding for server-side functions.
//!
//! This module owns outbound transactional email in Rust and is the sibling of
//! `runtime::imap`: IMAP reads a mailbox, this sends one message through a
//! tenant-configured HTTP email provider. Both follow the same shape — typed
//! config, typed errors, credentials that can never reach a log line.
//!
//! Three providers: two HTTP APIs (Resend, Brevo) and SMTP. SMTP opens a raw
//! TCP session, which is why it lives HERE and not in the sandbox: the function
//! never holds the socket, the credential or the host — it names a provider and
//! this module dials it, under the same egress rules as every other outbound
//! call.
//!
//! A tenant may configure SEVERAL providers and mark one default. The default
//! is what every system-originated mail uses (magic link above all), and a
//! function may name another with `raisin.email.send({ provider: "..." })`.
//! Resolution is [`EmailConfig::resolve`] — ONE implementation, shared by the
//! function binding and anything else that sends, so the console's idea of
//! "the default" and the send path's can never drift.
//!
//! Security:
//! * The credential lives in the secret store and is passed to [`send`] as a
//!   short-lived borrow. Every type that can hold it has a hand-written `Debug`
//!   that redacts it, and no error string ever embeds it — a provider's 401 is
//!   reported by status, never by echoing what was sent.
//! * `base_url` is an explicit property of the `/config/email` node. It is NOT
//!   derived from a request `Host`/`Origin` header: a link built from an
//!   attacker-controlled header is a poisoned link that still carries a valid
//!   token.
//!
//! [`send`]: EmailProvider::send

mod attachment;
mod brevo;
mod provider;
mod resend;
mod smtp;

pub use attachment::{
    AttachmentLimits, PartialAttachmentLimits, ResolvedAttachment, MAX_ATTACHMENTS,
    MAX_ATTACHMENTS_TOTAL_BYTES, MAX_ATTACHMENT_BYTES,
};
pub use provider::{EmailProvider, SendReceipt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Connect timeout for a provider request.
pub const CONNECT_TIMEOUT_SECS: u64 = 10;
/// Overall per-send timeout (connect + request + response).
pub const SEND_TIMEOUT_SECS: u64 = 30;
/// Hard cap on recipients in a single send. Transactional mail is one-to-one or
/// one-to-a-handful; anything larger is a bulk send that belongs to a campaign
/// tool, and an unbounded list is an amplification primitive.
pub const MAX_RECIPIENTS: usize = 20;

/// The parsed form of the `raisin:EmailConfig` node at `/config/email` in the
/// `raisin:system` workspace. Per tenant — there is no global email config.
///
/// The credential itself is NOT part of this struct: the node only carries a
/// reference (`secret://email/api_key`), and the resolved secret is passed to
/// [`EmailProvider::send`] separately so it never sits in a long-lived,
/// cloneable, serializable value.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EmailConfig {
    /// Master switch for the whole tenant. Off means no provider sends,
    /// however many are configured.
    #[serde(default)]
    pub enabled: bool,
    /// Absolute base URL of this tenant's front end (e.g.
    /// `https://app.example.com`), used to build magic links. Explicit on
    /// purpose: see the module-level note on link poisoning.
    #[serde(default)]
    pub base_url: String,
    /// The configured senders. Each entry is complete on its own: provider
    /// API, sender identity, credential reference and — for SMTP — where to
    /// connect.
    ///
    /// Empty means this tenant cannot send. There is no implicit sender and no
    /// fallback account: mail either goes out as somebody this tenant
    /// configured, or it does not go out.
    #[serde(default)]
    pub providers: Vec<EmailProviderConfig>,
    /// Name of the provider system mail goes through. Optional: with a single
    /// configured provider, or one carrying `default: true`, it is redundant.
    #[serde(default)]
    pub default_provider: Option<String>,
}

/// One configured sender inside [`EmailConfig::providers`].
///
/// `name` is how a function asks for it and how the console labels it; it is
/// NOT the provider API. Two Resend accounts (marketing and transactional, say)
/// are two entries with the same `provider` and different names.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EmailProviderConfig {
    /// Slug a caller names, unique within the tenant. Compared
    /// case-insensitively after trimming.
    pub name: String,
    /// Which provider API this entry talks to.
    pub provider: EmailProvider,
    /// Envelope sender address, verified with THIS entry's account.
    #[serde(default)]
    pub from_address: String,
    /// Optional display name rendered alongside `from_address`.
    #[serde(default)]
    pub from_name: Option<String>,
    /// Optional `Reply-To` address.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Where this entry's credential lives in the secret store. Each entry
    /// carries its own, so rotating one account cannot disturb another.
    #[serde(default = "default_credential_ref")]
    pub credential_ref: String,
    /// Override for the provider's API base URL. Ignored by SMTP.
    #[serde(default)]
    pub api_base: Option<String>,
    /// Connection settings, required when `provider` is `smtp` and unused
    /// otherwise.
    #[serde(default)]
    pub smtp: Option<SmtpSettings>,
    /// Per-entry switch. A disabled entry cannot be selected — not by name and
    /// not as the default — so an account can be parked without deleting its
    /// configuration.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Marks the entry system mail goes through. `default` is a Rust keyword,
    /// hence the field name; the wire name stays `default`.
    #[serde(default, rename = "default")]
    pub is_default: bool,
    /// Per-entry attachment bounds. Absent fields keep the defaults.
    ///
    /// Raising `max_total_bytes` here without also raising
    /// [`SEND_TIMEOUT_SECS`] trades a rejected message for a timed-out one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<PartialAttachmentLimits>,
}

/// How an SMTP session is secured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpSecurity {
    /// Connect in the clear, then `STARTTLS`. The submission default (587).
    #[default]
    Starttls,
    /// Implicit TLS from the first byte (465).
    Tls,
    /// No encryption at all. Only ever appropriate for a relay on a trusted
    /// network — the credential crosses the wire in the clear.
    None,
}

/// SMTP connection settings.
///
/// The PASSWORD is not here. Like every other provider, the credential is a
/// `secret://` reference on the entry, so an SMTP config can be read, diffed
/// and audited without exposing the account.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SmtpSettings {
    /// Submission host, e.g. `smtp-relay.brevo.com`.
    pub host: String,
    /// Submission port. 587 (STARTTLS) unless stated otherwise.
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    /// Login username. Often the account email; for Brevo's relay it is the
    /// account login, and the SMTP key is the password.
    #[serde(default)]
    pub username: Option<String>,
    /// Transport security.
    #[serde(default)]
    pub security: SmtpSecurity,
}

impl std::fmt::Display for SmtpSecurity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SmtpSecurity::Starttls => "starttls",
            SmtpSecurity::Tls => "tls",
            SmtpSecurity::None => "none",
        })
    }
}

fn default_smtp_port() -> u16 {
    587
}

/// One resolved sender: everything a provider module needs, and nothing about
/// which of the two config models it came from.
///
/// This is the ONLY shape the provider modules see. Adding a config model (as
/// `providers` was added beside the legacy fields) therefore cannot fork the
/// send path — it adds a way to build this struct, not a second sender.
#[derive(Clone, Debug)]
pub struct EmailSender {
    /// The name this sender was resolved under, echoed in the receipt so a
    /// caller can tell which of several accounts actually sent.
    pub name: String,
    /// Which provider API to talk to.
    pub provider: EmailProvider,
    /// Envelope sender address.
    pub from_address: String,
    /// Optional display name.
    pub from_name: Option<String>,
    /// Optional `Reply-To`.
    pub reply_to: Option<String>,
    /// Secret-store reference for this sender's credential.
    pub credential_ref: String,
    /// Provider API base override.
    pub api_base: Option<String>,
    /// SMTP connection settings, present only for [`EmailProvider::Smtp`].
    pub smtp: Option<SmtpSettings>,
    /// Attachment bounds for this sender: the defaults, with this entry's
    /// overrides applied. Resolved here so a provider module can never see an
    /// unresolved limit.
    pub attachment_limits: AttachmentLimits,
}

/// A minimal sender for tests.
///
/// Exists so that adding a field to [`EmailSender`] does not break every test
/// literal in this module — the previous round of that is why this is here.
/// Test-only on purpose: in production a sender is built by
/// [`EmailConfig::senders`], and a default-constructible one would make it
/// possible to send from a half-configured value.
#[cfg(test)]
impl Default for EmailSender {
    fn default() -> Self {
        Self {
            name: "primary".to_string(),
            provider: EmailProvider::Resend,
            from_address: "noreply@example.com".to_string(),
            from_name: None,
            reply_to: None,
            credential_ref: "secret://email/api_key".to_string(),
            api_base: None,
            smtp: None,
            attachment_limits: AttachmentLimits::default(),
        }
    }
}

impl EmailSender {
    /// `From` header value, `"Name <addr>"` when a display name is configured
    /// and the bare address otherwise.
    pub fn from_header(&self) -> String {
        match &self.from_name {
            Some(name) if !name.trim().is_empty() => format!("{name} <{}>", self.from_address),
            _ => self.from_address.clone(),
        }
    }

    /// Refuse a sender that cannot possibly send, before a credential is
    /// decrypted or a socket opened.
    fn validate(&self) -> Result<()> {
        if self.from_address.trim().is_empty() {
            return Err(EmailError::Config(format!(
                "email provider `{}` has no from_address",
                self.name
            )));
        }
        if self.provider == EmailProvider::Smtp {
            let smtp = self.smtp.as_ref().ok_or_else(|| {
                EmailError::Config(format!(
                    "email provider `{}` selects smtp but carries no smtp settings                      (host, port, username, security)",
                    self.name
                ))
            })?;
            if smtp.host.trim().is_empty() {
                return Err(EmailError::Config(format!(
                    "email provider `{}` has an empty smtp host",
                    self.name
                )));
            }
            if smtp.port == 0 {
                return Err(EmailError::Config(format!(
                    "email provider `{}` has an invalid smtp port 0",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

fn default_credential_ref() -> String {
    "secret://email/api_key".to_string()
}
fn default_true() -> bool {
    true
}

impl EmailConfig {
    /// Parse the config from the node's property map.
    pub fn from_value(v: &Value) -> Result<Self> {
        serde_json::from_value(v.clone())
            .map_err(|e| EmailError::Config(format!("invalid email config: {e}")))
    }

    /// Every configured sender, in configuration order, each paired with
    /// `(enabled, is_default)`.
    ///
    /// Disabled entries are included — the console lists them, and a by-name
    /// request must be able to answer "that one is disabled" rather than "that
    /// one does not exist". [`resolve`] is what filters.
    ///
    /// [`resolve`]: Self::resolve
    pub fn senders(&self) -> Vec<(EmailSender, bool, bool)> {
        // An explicit `default_provider` wins over per-entry `default` flags,
        // so the two spellings cannot disagree: naming one is how you move the
        // default without editing two entries.
        let explicit_default = self
            .default_provider
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty());

        self.providers
            .iter()
            .map(|p| {
                let is_default = match explicit_default {
                    Some(name) => name.eq_ignore_ascii_case(p.name.trim()),
                    None => p.is_default,
                };
                (
                    EmailSender {
                        name: p.name.trim().to_string(),
                        provider: p.provider,
                        from_address: p.from_address.trim().to_string(),
                        from_name: p.from_name.clone(),
                        reply_to: p.reply_to.clone(),
                        credential_ref: p.credential_ref.trim().to_string(),
                        api_base: p.api_base.clone(),
                        smtp: p.smtp.clone(),
                        attachment_limits: AttachmentLimits::default()
                            .merged_with(p.attachments.as_ref()),
                    },
                    p.enabled,
                    is_default,
                )
            })
            .collect()
    }

    /// Pick the sender for one send.
    ///
    /// `requested` is what a caller named, or `None` for "whatever this tenant
    /// sends system mail through". The rules, in order:
    ///
    /// 1. A named provider must exist and be enabled. A typo is an error that
    ///    lists what IS configured, never a silent fall-through to the default
    ///    — mail leaving through the wrong account is worse than mail not
    ///    leaving.
    /// 2. Unnamed picks `default_provider`, else the entry flagged `default`,
    ///    else the only enabled entry.
    /// 3. Several enabled entries and no default is a configuration error, for
    ///    the same reason as (1).
    ///
    /// This is the ONE place either question is answered.
    pub fn resolve(&self, requested: Option<&str>) -> Result<EmailSender> {
        if !self.enabled {
            return Err(EmailError::Config(
                "outbound email is not enabled for this tenant \
                 (set enabled: true on /config/email)"
                    .to_string(),
            ));
        }

        let all = self.senders();
        if all.is_empty() {
            return Err(EmailError::Config(
                "no email provider is configured for this tenant \
                 (add one on /config/email)"
                    .to_string(),
            ));
        }

        // Every "which one did you mean" error lists what IS configured: the
        // whole class of failure here is a name that does not match, and an
        // error that does not show the alternatives sends the reader to the
        // console to find out.
        let known = || -> String {
            let names: Vec<&str> = all.iter().map(|(s, _, _)| s.name.as_str()).collect();
            format!("configured providers: {names:?}")
        };

        let sender = match requested.map(str::trim).filter(|r| !r.is_empty()) {
            Some(want) => {
                let (sender, enabled, _) = all
                    .iter()
                    .find(|(s, _, _)| s.name.eq_ignore_ascii_case(want))
                    .ok_or_else(|| {
                        EmailError::Config(format!("unknown email provider `{want}`; {}", known()))
                    })?;
                if !*enabled {
                    return Err(EmailError::Config(format!(
                        "email provider `{want}` is disabled"
                    )));
                }
                sender.clone()
            }
            None => {
                let mut enabled = all.iter().filter(|(_, e, _)| *e);
                let defaulted: Vec<&(EmailSender, bool, bool)> =
                    all.iter().filter(|(_, e, d)| *e && *d).collect();
                match defaulted.len() {
                    1 => defaulted[0].0.clone(),
                    0 => {
                        let first = enabled.next().ok_or_else(|| {
                            EmailError::Config(
                                "every configured email provider is disabled".to_string(),
                            )
                        })?;
                        if enabled.next().is_some() {
                            return Err(EmailError::Config(format!(
                                "several email providers are enabled but none is marked \
                                 default; set default_provider on /config/email. {}",
                                known()
                            )));
                        }
                        first.0.clone()
                    }
                    _ => {
                        return Err(EmailError::Config(format!(
                            "several email providers are marked default; exactly one may be. {}",
                            known()
                        )))
                    }
                }
            }
        };

        sender.validate()?;
        Ok(sender)
    }
}

/// A resolved provider credential.
///
/// Exists so that the secret is carried by a type that cannot be printed: the
/// `Debug` impl is hand-written (the [`super::imap::ImapConn`] precedent) and
/// there is no `Display`, no `Serialize`, and no public field. The only way out
/// is [`Credential::expose`], which is deliberately noisy at the call site.
#[derive(Clone)]
pub struct Credential(String);

impl Credential {
    /// Wrap a resolved secret.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The raw secret, for placing into an outbound `Authorization` / `api-key`
    /// header. Never log the result.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

// Redact so the credential can never leak through a `{:?}` / tracing render.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

impl From<&str> for Credential {
    fn from(s: &str) -> Self {
        Credential::new(s)
    }
}

/// One outbound message. `from` / `from_name` / `reply_to` are intentionally
/// absent: they come from [`EmailConfig`], so a function author cannot forge a
/// sender the tenant has not verified.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct EmailMessage {
    /// One or more primary recipient addresses. At least one; `to`, `cc` and
    /// `bcc` together are capped at [`MAX_RECIPIENTS`].
    pub to: Vec<String>,
    /// Carbon-copy recipients, visible to everyone who receives the message.
    #[serde(default)]
    pub cc: Vec<String>,
    /// Blind-carbon-copy recipients. Each is invisible to every other
    /// recipient — see [`EmailMessage::bcc`]'s handling in the SMTP module,
    /// where the header must be dropped before the message goes on the wire.
    #[serde(default)]
    pub bcc: Vec<String>,
    /// Subject line.
    pub subject: String,
    /// Plain-text body. Always required — an HTML-only message is a spam signal
    /// and unreadable in a text client.
    pub text: String,
    /// Optional HTML body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// Attachments, already resolved to bytes.
    ///
    /// `skip` is not tidiness, it is the enforcement point. This struct is
    /// built with `serde_json::from_value` on a caller-supplied object, so a
    /// deserializable field here would let a function post raw bytes and an
    /// arbitrary content type straight into a message — bypassing every
    /// sanitiser, every size cap and the entire source model. Attachments may
    /// only arrive through the resolution layer, which is what
    /// `skip_deserializing` guarantees. `skip_serializing` keeps a 10 MiB
    /// payload out of anything that logs the message.
    #[serde(skip)]
    pub attachments: Vec<ResolvedAttachment>,
}

impl EmailMessage {
    /// Build a plain-text message to a single recipient.
    pub fn new(to: impl Into<String>, subject: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            to: vec![to.into()],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.into(),
            text: text.into(),
            html: None,
            attachments: Vec::new(),
        }
    }

    /// Attach an HTML alternative body.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }

    /// Attach already-resolved attachments.
    pub fn with_attachments(mut self, attachments: Vec<ResolvedAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    /// Every recipient this message will be delivered to, across all three
    /// fields.
    ///
    /// One definition, because both the cap and the `email_policy` check must
    /// see the same set: a policy that walked only `to` would make `bcc` a
    /// one-line bypass of the entire allowlist.
    pub fn all_recipients(&self) -> impl Iterator<Item = &String> {
        self.to.iter().chain(&self.cc).chain(&self.bcc)
    }

    /// Total recipients across `to`, `cc` and `bcc`.
    pub fn recipient_count(&self) -> usize {
        self.to.len() + self.cc.len() + self.bcc.len()
    }

    /// Reject a message the provider would either bounce or, worse, accept with
    /// smuggled headers.
    ///
    /// Both providers take JSON rather than a raw RFC 5322 blob, so a CR/LF in
    /// a subject or address is not a header-injection vector *for them* — but it
    /// is for any future SMTP provider, and a header-splitting attempt is never
    /// legitimate traffic. Rejecting here keeps the check in one place.
    pub(crate) fn validate(&self) -> Result<()> {
        // `to` specifically, not the union: a message addressed only via bcc
        // is accepted by some providers and rejected by others, and "no
        // visible recipient" is not a shape worth supporting differently per
        // provider.
        if self.to.is_empty() {
            return Err(EmailError::InvalidMessage("no recipients".to_string()));
        }
        // The cap is on the union. Per-field caps would silently make the real
        // ceiling three times what the constant says.
        if self.recipient_count() > MAX_RECIPIENTS {
            return Err(EmailError::InvalidMessage(format!(
                "too many recipients: {} across to/cc/bcc (max {MAX_RECIPIENTS})",
                self.recipient_count()
            )));
        }
        for addr in self.all_recipients() {
            if addr.trim().is_empty() || !addr.contains('@') {
                return Err(EmailError::InvalidMessage(
                    "recipient is not an email address".to_string(),
                ));
            }
            if has_control_chars(addr) {
                return Err(EmailError::InvalidMessage(
                    "recipient contains control characters".to_string(),
                ));
            }
        }
        self.validate_attachment_shape()?;
        if self.subject.is_empty() {
            return Err(EmailError::InvalidMessage("empty subject".to_string()));
        }
        if has_control_chars(&self.subject) {
            return Err(EmailError::InvalidMessage(
                "subject contains control characters".to_string(),
            ));
        }
        if self.text.is_empty() {
            return Err(EmailError::InvalidMessage("empty text body".to_string()));
        }
        Ok(())
    }

    /// Attachment checks that need nothing but the message itself.
    ///
    /// Size and count live in [`validate_attachments`] instead, because those
    /// depend on the resolved sender's limits.
    ///
    /// [`validate_attachments`]: Self::validate_attachments
    fn validate_attachment_shape(&self) -> Result<()> {
        let mut seen_ids: Vec<&str> = Vec::new();
        for a in &self.attachments {
            if a.filename.trim().is_empty() && !a.is_inline() {
                return Err(EmailError::InvalidMessage(
                    "attachment has an empty filename".to_string(),
                ));
            }
            let Some(cid) = a.content_id.as_deref() else {
                continue;
            };
            // An inline part is referenced from the HTML body. With no HTML
            // body there is nothing that could reference it, and the recipient
            // gets a mail whose image is simply missing.
            if self.html.as_deref().unwrap_or("").is_empty() {
                return Err(EmailError::InvalidMessage(format!(
                    "attachment `{}` is inline (content_id `{cid}`) but the message has no html body to reference it from",
                    a.filename
                )));
            }
            if seen_ids.contains(&cid) {
                return Err(EmailError::InvalidMessage(format!(
                    "duplicate attachment content_id `{cid}`: a cid: reference would be ambiguous"
                )));
            }
            seen_ids.push(cid);
        }
        Ok(())
    }

    /// Enforce the resolved sender's attachment bounds and the provider's
    /// capabilities.
    ///
    /// Separate from [`validate`] only because it needs the sender. Both run
    /// before a socket is opened.
    ///
    /// [`validate`]: Self::validate
    pub(crate) fn validate_attachments(
        &self,
        provider: EmailProvider,
        limits: &AttachmentLimits,
    ) -> Result<()> {
        if self.attachments.is_empty() {
            return Ok(());
        }
        if self.attachments.len() > limits.max_count {
            return Err(EmailError::InvalidMessage(format!(
                "too many attachments: {} (max {})",
                self.attachments.len(),
                limits.max_count
            )));
        }
        let mut total = 0usize;
        for a in &self.attachments {
            if a.bytes.len() > limits.max_bytes_each {
                return Err(EmailError::InvalidMessage(format!(
                    "attachment `{}` is {} bytes (max {} each)",
                    a.filename,
                    a.bytes.len(),
                    limits.max_bytes_each
                )));
            }
            total = total.saturating_add(a.bytes.len());
        }
        if total > limits.max_total_bytes {
            return Err(EmailError::InvalidMessage(format!(
                "attachments total {total} bytes (max {})",
                limits.max_total_bytes
            )));
        }
        // Refuse rather than degrade. Sending an inline part through a
        // provider that cannot carry a Content-ID produces a mail whose image
        // is a stray attachment and whose `cid:` reference resolves to
        // nothing — broken in the inbox, silent everywhere else.
        if !provider.supports_inline() {
            if let Some(a) = self.attachments.iter().find(|a| a.is_inline()) {
                return Err(EmailError::InvalidMessage(format!(
                    "provider `{}` does not support inline attachments (attachment `{}` sets content_id); \
                     send it as a normal attachment, or use an smtp or resend sender",
                    provider.as_str(),
                    a.filename
                )));
            }
        }
        Ok(())
    }
}

/// True when `s` contains a CR, LF or NUL — the header-splitting trio.
fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| c == '\r' || c == '\n' || c == '\0')
}

/// Typed email error. `code()` yields the stable machine code an adapter or an
/// HTTP layer maps onto. The `Display` / `Debug` text never contains the
/// credential nor the request body.
#[derive(Debug, thiserror::Error)]
pub enum EmailError {
    /// The provider rejected the credential (HTTP 401/403). Distinct from
    /// [`EmailError::InvalidMessage`] on purpose: one is an operator problem
    /// (rotate the secret), the other is a caller problem (fix the message).
    #[error("[email:auth_failed] {0}")]
    Auth(String),
    /// The provider rejected the message itself (HTTP 400/422) — unverified
    /// sender domain, malformed address, missing field — or local validation
    /// refused it before a socket was opened.
    #[error("[email:invalid_message] {0}")]
    InvalidMessage(String),
    /// The provider is throttling us (HTTP 429).
    #[error("[email:rate_limited] {0}")]
    RateLimited(String),
    /// The provider answered with a status this module does not special-case,
    /// or a body it could not parse.
    #[error("[email:provider_error] {0}")]
    Provider(String),
    /// DNS / TCP / TLS failure reaching the provider.
    #[error("[email:transport] {0}")]
    Transport(String),
    /// The send exceeded [`SEND_TIMEOUT_SECS`].
    #[error("[email:timeout] send timed out")]
    Timeout,
    /// Malformed `/config/email`, or email disabled for this tenant.
    #[error("[email:config] {0}")]
    Config(String),
    /// A provider variant that exists in the enum but has no implementation
    /// yet (today: SMTP).
    #[error("[email:unsupported] {0}")]
    Unsupported(String),
}

impl EmailError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            EmailError::Auth(_) => "auth_failed",
            EmailError::InvalidMessage(_) => "invalid_message",
            EmailError::RateLimited(_) => "rate_limited",
            EmailError::Provider(_) => "provider_error",
            EmailError::Transport(_) => "transport",
            EmailError::Timeout => "timeout",
            EmailError::Config(_) => "config",
            EmailError::Unsupported(_) => "unsupported",
        }
    }
}

impl From<EmailError> for raisin_error::Error {
    fn from(e: EmailError) -> Self {
        let msg = e.to_string();
        match e {
            // An auth failure here is *our* credential failing against the
            // provider, not the caller's — it must not be rendered to an end
            // user as "you are unauthorized".
            EmailError::Auth(_) => raisin_error::Error::Backend(msg),
            EmailError::InvalidMessage(_) => raisin_error::Error::Validation(msg),
            EmailError::Config(_) => raisin_error::Error::InvalidState(msg),
            _ => raisin_error::Error::Backend(msg),
        }
    }
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, EmailError>;

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "re_supersecret_ABC123";

    fn entry(name: &str, provider: EmailProvider) -> EmailProviderConfig {
        EmailProviderConfig {
            name: name.to_string(),
            provider,
            from_address: format!("{name}@example.com"),
            from_name: Some("Example".to_string()),
            reply_to: None,
            credential_ref: format!("secret://email/{name}"),
            api_base: None,
            smtp: None,
            enabled: true,
            is_default: false,
            attachments: None,
        }
    }

    fn cfg(providers: Vec<EmailProviderConfig>) -> EmailConfig {
        EmailConfig {
            enabled: true,
            base_url: "https://app.example.com".to_string(),
            providers,
            default_provider: None,
        }
    }

    // ---- Credential redaction -------------------------------------------

    #[test]
    fn credential_is_redacted_in_debug() {
        let c = Credential::new(SECRET);
        let rendered = format!("{c:?}");
        assert!(
            !rendered.contains(SECRET),
            "credential leaked through Debug: {rendered}"
        );
        assert_eq!(rendered, "Credential(<redacted>)");
        // And still readable where it is actually needed.
        assert_eq!(c.expose(), SECRET);
    }

    #[test]
    fn nothing_reachable_from_a_send_renders_the_credential() {
        // Every value the send path constructs and might end up in a tracing
        // field or an error log, checked in one place.
        let c = Credential::new(SECRET);
        let config = cfg(vec![entry("primary", EmailProvider::Resend)]);
        let sender = config.resolve(None).expect("one provider resolves");
        let msg = EmailMessage::new("user@example.com", "Sign in", "link").with_html("<b>link</b>");
        let receipt = SendReceipt {
            message_id: "abc-123".to_string(),
            provider: EmailProvider::Resend,
            sender: "primary".to_string(),
        };
        let err = EmailError::Auth("provider returned 401".to_string());
        for rendered in [
            format!("{c:?}"),
            format!("{config:?}"),
            format!("{sender:?}"),
            format!("{msg:?}"),
            format!("{receipt:?}"),
            format!("{err:?}"),
            err.to_string(),
        ] {
            assert!(!rendered.contains(SECRET), "credential leaked: {rendered}");
        }
    }

    #[test]
    fn from_header_uses_display_name_when_present() {
        let sender = cfg(vec![entry("primary", EmailProvider::Resend)])
            .resolve(None)
            .expect("resolves");
        assert_eq!(sender.from_header(), "Example <primary@example.com>");

        let mut bare = sender.clone();
        bare.from_name = None;
        assert_eq!(bare.from_header(), "primary@example.com");
        // Whitespace is not a display name.
        bare.from_name = Some("   ".to_string());
        assert_eq!(bare.from_header(), "primary@example.com");
    }

    // ---- Message validation ---------------------------------------------

    #[test]
    fn validate_rejects_header_splitting_and_empty_fields() {
        let mut m = EmailMessage::new("user@example.com", "Sign in", "link");
        assert!(m.validate().is_ok());

        m.subject = "Sign in\r\nBcc: attacker@evil.test".to_string();
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage::new("user@example.com", "", "link");
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage::new("not-an-address", "Sign in", "link");
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage {
            to: vec![],
            subject: "Sign in".to_string(),
            text: "link".to_string(),
            ..Default::default()
        };
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));

        let m = EmailMessage {
            to: vec!["user@example.com".to_string(); MAX_RECIPIENTS + 1],
            subject: "Sign in".to_string(),
            text: "link".to_string(),
            ..Default::default()
        };
        assert!(matches!(m.validate(), Err(EmailError::InvalidMessage(_))));
    }

    // ---- Recipients and attachments --------------------------------------

    fn att(bytes: usize, cid: Option<&str>) -> ResolvedAttachment {
        ResolvedAttachment {
            filename: "ticket.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            bytes: vec![0u8; bytes],
            content_id: cid.map(str::to_string),
        }
    }

    /// The cap is on the union. Per-field caps would make the real ceiling
    /// three times what the constant advertises.
    #[test]
    fn the_recipient_cap_counts_to_cc_and_bcc_together() {
        let mut m = EmailMessage::new("user@example.com", "Subject", "body");
        m.cc = vec!["c@example.com".to_string(); 10];
        m.bcc = vec!["b@example.com".to_string(); 10];
        assert_eq!(m.recipient_count(), 21);
        let err = m.validate().expect_err("21 recipients must be refused");
        assert!(err.to_string().contains("across to/cc/bcc"), "{err}");

        m.bcc.pop();
        assert!(m.validate().is_ok(), "20 recipients is the cap, not 19");
    }

    /// Header splitting is refused wherever the address sits, not only in
    /// `to` — a `bcc` is just as much a header.
    #[test]
    fn a_hostile_address_is_refused_in_every_recipient_field() {
        for field in ["cc", "bcc"] {
            let mut m = EmailMessage::new("user@example.com", "Subject", "body");
            let bad = "x@example.com\r\nBcc: attacker@evil.test".to_string();
            match field {
                "cc" => m.cc = vec![bad],
                _ => m.bcc = vec![bad],
            }
            let err = m.validate().expect_err("{field} must be validated");
            assert!(err.to_string().contains("control characters"), "{err}");
        }
    }

    #[test]
    fn an_inline_attachment_needs_an_html_body_to_reference_it() {
        let m = EmailMessage::new("user@example.com", "Subject", "body")
            .with_attachments(vec![att(4, Some("logo"))]);
        let err = m.validate().expect_err("cid with no html must be refused");
        assert!(err.to_string().contains("no html body"), "{err}");
    }

    #[test]
    fn duplicate_content_ids_are_refused() {
        let m = EmailMessage::new("user@example.com", "Subject", "body")
            .with_html("<img src=\"cid:logo\">")
            .with_attachments(vec![att(4, Some("logo")), att(4, Some("logo"))]);
        let err = m.validate().expect_err("duplicate cid must be refused");
        assert!(err.to_string().contains("duplicate"), "{err}");
    }

    #[test]
    fn attachment_bounds_are_enforced_against_the_senders_limits() {
        let limits = AttachmentLimits {
            max_count: 2,
            max_bytes_each: 100,
            max_total_bytes: 150,
        };
        let msg = |n: usize, each: usize| {
            EmailMessage::new("user@example.com", "Subject", "body").with_attachments(vec![
                att(
                    each, None
                );
                n
            ])
        };

        let err = msg(3, 10)
            .validate_attachments(EmailProvider::Resend, &limits)
            .expect_err("count");
        assert!(err.to_string().contains("too many attachments"), "{err}");

        let err = msg(1, 200)
            .validate_attachments(EmailProvider::Resend, &limits)
            .expect_err("per item");
        assert!(err.to_string().contains("max 100 each"), "{err}");

        let err = msg(2, 90)
            .validate_attachments(EmailProvider::Resend, &limits)
            .expect_err("total");
        assert!(err.to_string().contains("total 180 bytes"), "{err}");

        assert!(msg(2, 50)
            .validate_attachments(EmailProvider::Resend, &limits)
            .is_ok());
    }

    /// Refused, never degraded: Brevo would accept the send and the recipient
    /// would get a mail with a broken image and a dangling `cid:` reference.
    #[test]
    fn brevo_refuses_an_inline_attachment_and_says_what_to_use_instead() {
        let m = EmailMessage::new("user@example.com", "Subject", "body")
            .with_html("<img src=\"cid:logo\">")
            .with_attachments(vec![att(4, Some("logo"))]);

        let err = m
            .validate_attachments(EmailProvider::Brevo, &AttachmentLimits::default())
            .expect_err("brevo cannot carry a content_id");
        let msg = err.to_string();
        assert!(msg.contains("brevo"), "{msg}");
        assert!(msg.contains("smtp") && msg.contains("resend"), "{msg}");

        // The very same message is fine on the providers that can carry it.
        for p in [EmailProvider::Smtp, EmailProvider::Resend] {
            assert!(m
                .validate_attachments(p, &AttachmentLimits::default())
                .is_ok());
        }
    }

    /// A caller must not be able to post resolved bytes straight into the
    /// message: that would skip every sanitiser and every size bound.
    #[test]
    fn attachments_cannot_be_deserialized_onto_the_message() {
        let m: EmailMessage = serde_json::from_value(serde_json::json!({
            "to": ["user@example.com"],
            "subject": "Subject",
            "text": "body",
            "attachments": [{
                "filename": "evil.sh",
                "content_type": "text/x-shellscript",
                "bytes": [1, 2, 3],
            }],
        }))
        .expect("the unknown key is ignored, not an error");
        assert!(
            m.attachments.is_empty(),
            "attachments must only arrive through resolution"
        );
    }

    // ---- Config parsing --------------------------------------------------

    #[test]
    fn config_parses_from_node_properties() {
        let v = serde_json::json!({
            "enabled": true,
            "base_url": "https://app.example.com",
            "default_provider": "brevo-api",
            "providers": [
                {
                    "name": "brevo-api",
                    "provider": "brevo",
                    "from_address": "noreply@example.com",
                },
                {
                    "name": "relay",
                    "provider": "smtp",
                    "from_address": "noreply@example.com",
                    "enabled": false,
                    "smtp": { "host": "smtp-relay.brevo.com", "username": "acct" },
                },
            ],
        });
        let parsed = EmailConfig::from_value(&v).expect("valid config");
        assert_eq!(parsed.providers.len(), 2);
        assert_eq!(parsed.providers[0].provider, EmailProvider::Brevo);
        // The credential_ref default is per entry, not per config.
        assert_eq!(parsed.providers[0].credential_ref, "secret://email/api_key");
        // `enabled` defaults to true on an entry (the tenant added it on
        // purpose) and to FALSE on the config (the master switch must be
        // deliberate).
        assert!(parsed.providers[0].enabled);
        assert!(!parsed.providers[1].enabled);
        // SMTP defaults: submission port, STARTTLS.
        let smtp = parsed.providers[1].smtp.as_ref().expect("smtp settings");
        assert_eq!(smtp.port, 587);
        assert_eq!(smtp.security, SmtpSecurity::Starttls);

        // An unknown provider is a config error, not a panic.
        let bad = serde_json::json!({
            "enabled": true,
            "providers": [{ "name": "x", "provider": "carrier_pigeon" }],
        });
        assert!(matches!(
            EmailConfig::from_value(&bad),
            Err(EmailError::Config(_))
        ));
    }

    /// A node with nothing but the master switch must parse. This is the state
    /// a freshly seeded tenant is in, and a parse error there would report
    /// "invalid config" for a config that is merely empty.
    #[test]
    fn an_unconfigured_node_parses_and_refuses_to_resolve() {
        let parsed = EmailConfig::from_value(&serde_json::json!({ "enabled": false }))
            .expect("an empty config is not a malformed one");
        assert!(parsed.providers.is_empty());
        assert_eq!(parsed.resolve(None).unwrap_err().code(), "config");
    }

    // ---- Resolution ------------------------------------------------------

    #[test]
    fn a_single_provider_is_the_default_without_being_marked() {
        let sender = cfg(vec![entry("only", EmailProvider::Resend)])
            .resolve(None)
            .expect("the only provider is the default");
        assert_eq!(sender.name, "only");
    }

    #[test]
    fn the_default_flag_picks_among_several() {
        let mut second = entry("marketing", EmailProvider::Brevo);
        second.is_default = true;
        let sender = cfg(vec![entry("transactional", EmailProvider::Resend), second])
            .resolve(None)
            .expect("the flagged provider is the default");
        assert_eq!(sender.name, "marketing");
        assert_eq!(sender.provider, EmailProvider::Brevo);
    }

    /// `default_provider` names the default outright, and must WIN over a
    /// stale per-entry flag — otherwise moving the default means editing two
    /// places and the two spellings can disagree.
    #[test]
    fn default_provider_overrides_a_stale_entry_flag() {
        let mut flagged = entry("old", EmailProvider::Resend);
        flagged.is_default = true;
        let mut config = cfg(vec![flagged, entry("new", EmailProvider::Brevo)]);
        config.default_provider = Some("new".to_string());
        assert_eq!(config.resolve(None).expect("resolves").name, "new");
    }

    /// Several enabled providers and no default is refused, not guessed. Mail
    /// leaving through the wrong account is worse than mail not leaving.
    #[test]
    fn several_providers_with_no_default_is_an_error() {
        let config = cfg(vec![
            entry("a", EmailProvider::Resend),
            entry("b", EmailProvider::Brevo),
        ]);
        let err = config.resolve(None).expect_err("ambiguous default");
        assert_eq!(err.code(), "config");
        // The error names the alternatives, so the reader does not have to open
        // the console to find out what exists.
        let text = err.to_string();
        assert!(text.contains("\"a\""), "{text}");
        assert!(text.contains("\"b\""), "{text}");
    }

    #[test]
    fn two_providers_marked_default_is_an_error() {
        let mut a = entry("a", EmailProvider::Resend);
        a.is_default = true;
        let mut b = entry("b", EmailProvider::Brevo);
        b.is_default = true;
        assert_eq!(cfg(vec![a, b]).resolve(None).unwrap_err().code(), "config");
    }

    /// A disabled entry is invisible to the default rules — which is what makes
    /// disabling one a safe way to park an account.
    #[test]
    fn a_disabled_provider_is_skipped_by_the_default_rules() {
        let mut disabled = entry("parked", EmailProvider::Brevo);
        disabled.enabled = false;
        let config = cfg(vec![entry("live", EmailProvider::Resend), disabled]);
        assert_eq!(config.resolve(None).expect("resolves").name, "live");
    }

    #[test]
    fn a_named_provider_is_selected_and_matched_case_insensitively() {
        let config = cfg(vec![
            entry("transactional", EmailProvider::Resend),
            entry("Marketing", EmailProvider::Brevo),
        ]);
        let sender = config.resolve(Some("marketing")).expect("named");
        assert_eq!(sender.provider, EmailProvider::Brevo);
        assert_eq!(sender.credential_ref, "secret://email/Marketing");
    }

    /// A typo must NOT fall through to the default. Silently sending through
    /// another account is the failure this rule exists to prevent, and the
    /// error lists what is configured.
    #[test]
    fn an_unknown_provider_name_never_falls_back_to_the_default() {
        let mut only = entry("transactional", EmailProvider::Resend);
        only.is_default = true;
        let err = cfg(vec![only])
            .resolve(Some("transctional"))
            .expect_err("a typo is an error");
        assert_eq!(err.code(), "config");
        assert!(err.to_string().contains("transactional"));
    }

    #[test]
    fn naming_a_disabled_provider_says_so() {
        let mut disabled = entry("parked", EmailProvider::Brevo);
        disabled.enabled = false;
        let err = cfg(vec![entry("live", EmailProvider::Resend), disabled])
            .resolve(Some("parked"))
            .expect_err("disabled");
        assert!(err.to_string().contains("disabled"), "{err}");
    }

    /// The master switch outranks everything, including a by-name request.
    #[test]
    fn the_master_switch_refuses_even_a_named_provider() {
        let mut config = cfg(vec![entry("live", EmailProvider::Resend)]);
        config.enabled = false;
        assert_eq!(
            config.resolve(Some("live")).unwrap_err().code(),
            "config",
            "a disabled tenant must not send through any provider"
        );
    }

    /// An empty / whitespace `provider` argument means "the default" — a
    /// caller that passes `""` from an unset template variable must not get an
    /// "unknown provider ``" error.
    #[test]
    fn an_empty_provider_name_means_the_default() {
        let config = cfg(vec![entry("only", EmailProvider::Resend)]);
        assert_eq!(config.resolve(Some("  ")).expect("resolves").name, "only");
    }

    /// Resolution refuses a sender that cannot send, so the failure names the
    /// missing field instead of arriving as a provider 400 (or, for SMTP, as a
    /// panic on the absent settings).
    #[test]
    fn resolution_refuses_an_incomplete_sender() {
        let mut blank = entry("blank", EmailProvider::Resend);
        blank.from_address = String::new();
        assert!(cfg(vec![blank])
            .resolve(None)
            .unwrap_err()
            .to_string()
            .contains("from_address"));

        let mut smtpless = entry("relay", EmailProvider::Smtp);
        smtpless.smtp = None;
        assert!(cfg(vec![smtpless])
            .resolve(None)
            .unwrap_err()
            .to_string()
            .contains("smtp settings"));
    }

    /// `senders()` is what the console lists from; it must show disabled
    /// entries (so they can be re-enabled) and agree with `resolve` about which
    /// one is the default.
    #[test]
    fn senders_lists_disabled_entries_and_agrees_about_the_default() {
        let mut parked = entry("parked", EmailProvider::Brevo);
        parked.enabled = false;
        let mut config = cfg(vec![entry("live", EmailProvider::Resend), parked]);
        config.default_provider = Some("live".to_string());

        let listed = config.senders();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].0.name, "parked");
        assert!(!listed[1].1, "parked is disabled");

        let default_named: Vec<&str> = listed
            .iter()
            .filter(|(_, _, d)| *d)
            .map(|(s, _, _)| s.name.as_str())
            .collect();
        assert_eq!(default_named, vec!["live"]);
        assert_eq!(config.resolve(None).expect("resolves").name, "live");
    }
}
