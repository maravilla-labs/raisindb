// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Resolved attachments and the limits that bound them.
//!
//! This module holds the shape the provider modules see: bytes, already
//! fetched and already validated. Turning a caller's *source* (base64, a node
//! reference, a `Resource`) into these bytes happens exactly once, one layer
//! up in `api/raisindb/email.rs` — which is the layer that has storage and
//! row-level security. Keeping resolution out of here is what stops
//! `runtime/email` from growing a storage dependency, and what stops the three
//! providers from each learning their own way to fetch a blob.
//!
//! Two things in here are load-bearing and easy to undo by accident:
//!
//! * [`ResolvedAttachment`]'s `Debug` is hand-written. The derived one would
//!   render every byte of a 10 MiB PDF into a log line the first time a send
//!   fails. Same reason [`super::Credential`] has one.
//! * `content_id` is stored WITHOUT angle brackets. lettre adds them itself
//!   (`ContentId::from(format!("<{id}>"))`), so a stored `<logo>` becomes
//!   `Content-ID: <<logo>>` and every mail client silently fails to match
//!   `src="cid:logo"` — an inline image that is broken in the inbox and
//!   perfect in every test that only checks the header exists.

use serde::{Deserialize, Serialize};

/// Default cap on attachments per message.
pub const MAX_ATTACHMENTS: usize = 20;
/// Default cap on one attachment's decoded size.
pub const MAX_ATTACHMENT_BYTES: usize = 10 * 1024 * 1024;
/// Default cap on the decoded size of all attachments in one message.
///
/// Deliberately equal to [`MAX_ATTACHMENT_BYTES`] rather than a multiple of it:
/// the ceiling that matters is what one send holds in memory, and a send holds
/// roughly three copies (the JSON `Value` carrying base64, the decoded bytes,
/// and the provider's re-encoded request body). It is also coupled to
/// [`SEND_TIMEOUT_SECS`](super::SEND_TIMEOUT_SECS): 10 MiB in 30 s needs a
/// sustained ~3 Mbit/s, and raising this without raising that manufactures
/// timeouts on legitimate sends.
pub const MAX_ATTACHMENTS_TOTAL_BYTES: usize = 10 * 1024 * 1024;

/// The bounds one sender applies to attachments.
///
/// Resolved — every field is populated. The per-provider override is
/// [`PartialAttachmentLimits`], merged in
/// [`EmailConfig::senders`](super::EmailConfig::senders) so that
/// there is one build site and a provider module can never see an
/// unresolved limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentLimits {
    /// Maximum number of attachments in one message.
    pub max_count: usize,
    /// Maximum decoded size of any single attachment.
    pub max_bytes_each: usize,
    /// Maximum decoded size of all attachments combined.
    pub max_total_bytes: usize,
}

impl Default for AttachmentLimits {
    fn default() -> Self {
        Self {
            max_count: MAX_ATTACHMENTS,
            max_bytes_each: MAX_ATTACHMENT_BYTES,
            max_total_bytes: MAX_ATTACHMENTS_TOTAL_BYTES,
        }
    }
}

impl AttachmentLimits {
    /// Apply a provider entry's overrides on top of the defaults.
    ///
    /// An absent field keeps the default, so an operator raising only
    /// `max_total_bytes` does not silently reset the other two.
    pub fn merged_with(self, over: Option<&PartialAttachmentLimits>) -> Self {
        let Some(o) = over else { return self };
        Self {
            max_count: o.max_count.unwrap_or(self.max_count),
            max_bytes_each: o.max_bytes_each.unwrap_or(self.max_bytes_each),
            max_total_bytes: o.max_total_bytes.unwrap_or(self.max_total_bytes),
        }
    }
}

/// Per-provider overrides, as written in a `/config/email` provider entry.
///
/// The `raisin:EmailConfig` nodetype is `strict: false` with an untyped
/// `providers` array, so this needs no schema migration.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct PartialAttachmentLimits {
    /// Overrides [`AttachmentLimits::max_count`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_count: Option<usize>,
    /// Overrides [`AttachmentLimits::max_bytes_each`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes_each: Option<usize>,
    /// Overrides [`AttachmentLimits::max_total_bytes`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_bytes: Option<usize>,
}

/// One attachment, resolved to bytes and ready for any provider.
///
/// Constructed only by the resolution layer. It deliberately carries no
/// storage key and no node path: by the time a value of this type exists the
/// read has already happened under the right authority, and a provider module
/// has no business being able to start another one.
#[derive(Clone)]
pub struct ResolvedAttachment {
    /// File name shown to the recipient. Already validated: no control
    /// characters, no path separators.
    pub filename: String,
    /// MIME type, already stripped of parameters and known to parse.
    pub content_type: String,
    /// The decoded bytes.
    pub bytes: Vec<u8>,
    /// When set, this part is inline and the value is referenced from the HTML
    /// body as `cid:{content_id}`. Stored WITHOUT angle brackets — see the
    /// module docs.
    pub content_id: Option<String>,
}

impl ResolvedAttachment {
    /// True when this part is embedded in the HTML body rather than listed as
    /// a download.
    pub fn is_inline(&self) -> bool {
        self.content_id.is_some()
    }
}

/// Renders shape, never content.
///
/// `EmailMessage` is `Debug`-rendered on several error paths; the derived
/// implementation would put a base64-sized wall of bytes into a log line, and
/// for an attachment that is a document someone paid for.
impl std::fmt::Debug for ResolvedAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedAttachment")
            .field("filename", &self.filename)
            .field("content_type", &self.content_type)
            .field("bytes", &format_args!("<{} bytes>", self.bytes.len()))
            .field("content_id", &self.content_id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(bytes: usize) -> ResolvedAttachment {
        ResolvedAttachment {
            filename: "ticket.pdf".to_string(),
            content_type: "application/pdf".to_string(),
            bytes: vec![0xffu8; bytes],
            content_id: None,
        }
    }

    /// The whole point of the hand-written `Debug`: a failing send must not
    /// write the attachment into the log.
    #[test]
    fn debug_renders_the_length_not_the_bytes() {
        let rendered = format!("{:?}", att(1024 * 1024));
        assert!(rendered.contains("<1048576 bytes>"), "{rendered}");
        assert!(rendered.contains("ticket.pdf"), "{rendered}");
        // 255 would appear as soon as the byte vector itself is rendered.
        assert!(!rendered.contains("255"), "bytes leaked into Debug");
        assert!(rendered.len() < 200, "Debug output grew with the payload");
    }

    #[test]
    fn an_absent_override_keeps_each_default() {
        assert_eq!(
            AttachmentLimits::default().merged_with(None),
            AttachmentLimits::default()
        );
    }

    /// Overriding one bound must not reset the others — an operator raising
    /// the total would otherwise silently drop the per-item cap back down.
    #[test]
    fn a_partial_override_touches_only_what_it_names() {
        let over = PartialAttachmentLimits {
            max_total_bytes: Some(40 * 1024 * 1024),
            ..Default::default()
        };
        let merged = AttachmentLimits::default().merged_with(Some(&over));
        assert_eq!(merged.max_total_bytes, 40 * 1024 * 1024);
        assert_eq!(merged.max_count, MAX_ATTACHMENTS);
        assert_eq!(merged.max_bytes_each, MAX_ATTACHMENT_BYTES);
    }

    #[test]
    fn inline_is_decided_by_the_content_id() {
        assert!(!att(1).is_inline());
        let mut inline = att(1);
        inline.content_id = Some("logo".to_string());
        assert!(inline.is_inline());
    }
}
