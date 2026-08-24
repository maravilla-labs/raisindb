// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Turning a caller's attachment *source* into bytes.
//!
//! This is the only place a source becomes an attachment, and it lives here
//! rather than in `runtime/email` for two reasons: this layer has storage and
//! row-level security, and `runtime/email` must stay pure transport so the
//! three providers cannot each grow their own way to fetch a blob.
//!
//! Two sources, and deliberately only two:
//!
//! * `content` — base64 (or a `data:` URL), for bytes the function already
//!   holds.
//! * `node` + `property` — a node reference, read through the ordinary node
//!   API so row-level security applies.
//!
//! A bare storage key is NOT a source. It would name a blob directly, with no
//! node to enforce anything against, and there is no way to tell from a key
//! whether the caller should be able to read it.
//!
//! **The node-ref read carries the FUNCTION's authority, not the end user's.**
//! A function running without an auth context (a trigger, a scheduled job)
//! reads as system, so it can attach a blob the person who triggered it could
//! not see. That is already true of `raisin.nodes.get`; what makes it sharper
//! here is that the same call mails the bytes out. Two things bound it: the
//! recipient list is checked against `email_policy` *before* any blob is read,
//! and only a `Resource` property with a storage key is attachable — so this
//! is "attach a file", not "read any property as a file".

use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;

use super::RaisinFunctionApi;
use crate::runtime::email::{AttachmentLimits, EmailError, ResolvedAttachment};
use raisin_error::Result;

/// Base64 decoder for attachment payloads.
///
/// Explicit rather than `STANDARD` so the two things that actually vary are
/// stated: padding is accepted either way (hand-rolled encoders differ), and
/// the alphabet is standard only — URL-safe input is a caller bug worth
/// surfacing rather than quietly accepting.
const B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Longest filename we will put in a MIME header.
const MAX_FILENAME_LEN: usize = 255;
/// Longest `content_id`.
const MAX_CONTENT_ID_LEN: usize = 128;

/// One attachment as the caller wrote it, before any bytes exist.
///
/// Untagged: the caller writes `{ content: ... }` or `{ node: ... }`, not a
/// discriminator. Exactly one must be present, which is checked explicitly
/// rather than left to serde — an untagged enum reports "did not match any
/// variant", which is useless for telling someone they set both.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AttachmentSpec {
    /// Base64 payload, or a `data:` URL.
    #[serde(default)]
    pub content: Option<String>,
    /// Path of a node carrying the file.
    #[serde(default)]
    pub node: Option<String>,
    /// Workspace of `node`. Defaults to the message's workspace-less lookup.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Property on `node` holding the resource. Defaults to `file`.
    #[serde(default)]
    pub property: Option<String>,
    /// Name shown to the recipient.
    #[serde(default)]
    pub filename: Option<String>,
    /// MIME type. Derived from the filename or the stored resource when absent.
    #[serde(default, alias = "contentType", alias = "mimeType")]
    pub content_type: Option<String>,
    /// Set to embed this part in the HTML body as `cid:{content_id}`.
    #[serde(default, alias = "contentId")]
    pub content_id: Option<String>,
}

/// Default property name on a node-ref attachment.
const DEFAULT_RESOURCE_PROPERTY: &str = "file";

fn invalid(msg: impl Into<String>) -> raisin_error::Error {
    raisin_error::Error::from(EmailError::InvalidMessage(msg.into()))
}

/// Lift `attachments` out of the message object and parse the specs.
///
/// Removed from the value rather than left for serde to ignore, for the same
/// reason `provider` is: the message type is deliberately permissive, so a key
/// left in place would be silently dropped. It also must not reach
/// `EmailMessage`, whose `attachments` field is resolved bytes — see the
/// `skip` on it.
pub(crate) fn take_attachment_specs(message: &mut Value) -> Result<Vec<AttachmentSpec>> {
    let Some(raw) = message
        .as_object_mut()
        .and_then(|obj| obj.remove("attachments"))
    else {
        return Ok(Vec::new());
    };
    match raw {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => {
            let mut specs = Vec::with_capacity(items.len());
            for (i, item) in items.into_iter().enumerate() {
                let spec: AttachmentSpec = serde_json::from_value(item).map_err(|e| {
                    invalid(format!(
                        "attachments[{i}] must be {{ content | node, filename?, content_type?, content_id? }}: {e}"
                    ))
                })?;
                specs.push(spec.validated(i)?);
            }
            Ok(specs)
        }
        _ => Err(invalid("attachments must be an array")),
    }
}

impl AttachmentSpec {
    /// Shape checks that need no I/O.
    ///
    /// Runs before authorization and before any byte is fetched, so hostile
    /// input is refused at the cheapest possible point — and stays testable
    /// without a mock API.
    fn validated(mut self, index: usize) -> Result<Self> {
        let has_content = self.content.is_some();
        let has_node = self.node.as_deref().is_some_and(|n| !n.trim().is_empty());
        match (has_content, has_node) {
            (true, true) => {
                return Err(invalid(format!(
                    "attachments[{index}] sets both `content` and `node`; an attachment has exactly one source"
                )))
            }
            (false, false) => {
                return Err(invalid(format!(
                    "attachments[{index}] has no source: set `content` (base64 or a data: URL) or `node`"
                )))
            }
            _ => {}
        }

        if let Some(name) = self.filename.as_deref() {
            self.filename = Some(check_filename(name, index)?);
        } else if has_content {
            // A node-ref can borrow the resource's own name; inline bytes
            // have nothing to fall back to.
            return Err(invalid(format!(
                "attachments[{index}] needs a `filename`: the recipient sees it, and it decides the content type"
            )));
        }

        if let Some(ct) = self.content_type.as_deref() {
            self.content_type = Some(check_content_type(ct, index)?);
        }
        if let Some(cid) = self.content_id.as_deref() {
            self.content_id = Some(check_content_id(cid, index)?);
        }
        Ok(self)
    }
}

/// Validate a caller-supplied filename.
///
/// Rejects rather than sanitizes. Silently renaming someone's file is a
/// surprise that surfaces in the recipient's inbox, where nobody can debug it.
fn check_filename(name: &str, index: usize) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(invalid(format!(
            "attachments[{index}] has an empty filename"
        )));
    }
    if trimmed.len() > MAX_FILENAME_LEN {
        return Err(invalid(format!(
            "attachments[{index}] filename is longer than {MAX_FILENAME_LEN} bytes"
        )));
    }
    // Control characters are the header-splitting vector. lettre happens to
    // percent-encode them, but the two JSON providers have no such guard, and
    // a filename with a newline in it is never legitimate.
    if trimmed
        .chars()
        .any(|c| (c as u32) < 0x20 || c as u32 == 0x7f)
    {
        return Err(invalid(format!(
            "attachments[{index}] filename contains control characters"
        )));
    }
    // Path separators matter at the RECIPIENT's client, some of which still
    // honour a path when saving. RFC 2231 encoding does not neutralise these
    // — they are printable ASCII — so this check is ours to make.
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains(':') {
        return Err(invalid(format!(
            "attachments[{index}] filename may not contain a path separator"
        )));
    }
    // A bare `.` or `..` names a directory, not a file. Traversal spellings
    // like `../x` are already refused above by the separator check.
    if trimmed == "." || trimmed == ".." {
        return Err(invalid(format!(
            "attachments[{index}] filename `{trimmed}` is not a file name"
        )));
    }
    Ok(trimmed.to_string())
}

/// Validate and normalise a MIME type.
///
/// Parameters are stripped whole. A caller-supplied `boundary=` on a
/// `multipart/*` type would let one leaf part declare an entire MIME subtree,
/// smuggling parts the server never validated into the message — which is why
/// `multipart` and `message` are refused outright.
fn check_content_type(ct: &str, index: usize) -> Result<String> {
    let base = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if base.is_empty() {
        return Err(invalid(format!(
            "attachments[{index}] has an empty content_type"
        )));
    }
    let top = base.split('/').next().unwrap_or("");
    if top == "multipart" || top == "message" {
        return Err(invalid(format!(
            "attachments[{index}] content_type `{base}` cannot be an attachment: \
             a multipart or message type would smuggle a MIME subtree into one part"
        )));
    }
    // One parser, the same one the SMTP path composes with, so a type that
    // validates here cannot fail there.
    lettre::message::header::ContentType::parse(&base).map_err(|e| {
        invalid(format!(
            "attachments[{index}] has an invalid content_type `{base}`: {e}"
        ))
    })?;
    Ok(base)
}

/// Validate a `content_id` and strip one layer of angle brackets.
///
/// Stored bare. lettre wraps it itself, so a stored `<logo>` becomes
/// `Content-ID: <<logo>>` and every client fails to match `src="cid:logo"` —
/// a broken image that no header-presence assertion would catch.
fn check_content_id(cid: &str, index: usize) -> Result<String> {
    let t = cid.trim();
    let bare = t
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(t);
    if bare.is_empty() {
        return Err(invalid(format!(
            "attachments[{index}] has an empty content_id"
        )));
    }
    if bare.len() > MAX_CONTENT_ID_LEN {
        return Err(invalid(format!(
            "attachments[{index}] content_id is longer than {MAX_CONTENT_ID_LEN} bytes"
        )));
    }
    if !bare
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '%' | '@'))
    {
        return Err(invalid(format!(
            "attachments[{index}] content_id `{bare}` may only contain letters, digits and .._-+%@"
        )));
    }
    Ok(bare.to_string())
}

/// Decode a `content` payload into bytes, plus the media type when the payload
/// was a `data:` URL.
///
/// `max_bytes` bounds the ENCODED length before any allocation: decoding first
/// and checking after would turn a 1 GiB string into a 750 MiB `Vec` on the
/// way to rejecting it.
fn decode_content(raw: &str, index: usize, max_bytes: usize) -> Result<(Vec<u8>, Option<String>)> {
    let (payload, media_type) = match raw.strip_prefix("data:") {
        None => (raw, None),
        Some(rest) => {
            let (meta, data) = rest
                .split_once(',')
                .ok_or_else(|| invalid(format!("attachments[{index}] is a malformed data: URL")))?;
            if !meta.trim_end().ends_with(";base64") {
                return Err(invalid(format!(
                    "attachments[{index}] data: URL must be base64-encoded"
                )));
            }
            let mt = meta.trim_end_matches(";base64").trim();
            (data, (!mt.is_empty()).then(|| mt.to_ascii_lowercase()))
        }
    };

    // 4 base64 chars per 3 bytes, plus padding and any line breaks the caller
    // left in. Generous on purpose: this is a cheap pre-filter, and the exact
    // bound is enforced on the decoded length.
    let ceiling = max_bytes / 3 * 4 + 8;
    if payload.len() > ceiling.saturating_add(payload.len() / 32) {
        return Err(invalid(format!(
            "attachments[{index}] is larger than the {max_bytes} byte limit"
        )));
    }

    let cleaned: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if cleaned.contains('-') || cleaned.contains('_') {
        return Err(invalid(format!(
            "attachments[{index}] looks URL-safe base64 encoded; standard base64 is required"
        )));
    }
    // Never echo the payload: it is megabytes long, and it may be exactly the
    // thing the caller should not have had.
    let bytes = B64
        .decode(cleaned.as_bytes())
        .map_err(|_| invalid(format!("attachments[{index}] content is not valid base64")))?;
    Ok((bytes, media_type))
}

/// Guess a content type from a filename, defaulting to octet-stream.
fn guess_content_type(filename: &str) -> String {
    mime_guess::from_path(filename)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

impl RaisinFunctionApi {
    /// Resolve every spec into bytes.
    ///
    /// The ONLY I/O in the attachment path, and it runs after the recipient
    /// policy and the sender have been settled — so a function that may not
    /// send never causes a blob to be read.
    pub(crate) async fn resolve_attachments(
        &self,
        specs: Vec<AttachmentSpec>,
        limits: &AttachmentLimits,
    ) -> Result<Vec<ResolvedAttachment>> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        // Counted here as well as in `validate_attachments` so a caller who
        // sent 500 node refs does not get 500 node reads before being told.
        if specs.len() > limits.max_count {
            return Err(invalid(format!(
                "too many attachments: {} (max {})",
                specs.len(),
                limits.max_count
            )));
        }

        let mut out = Vec::with_capacity(specs.len());
        for (i, spec) in specs.into_iter().enumerate() {
            out.push(self.resolve_one(spec, i, limits).await?);
        }
        Ok(out)
    }

    async fn resolve_one(
        &self,
        spec: AttachmentSpec,
        index: usize,
        limits: &AttachmentLimits,
    ) -> Result<ResolvedAttachment> {
        let (bytes, filename, content_type) = match (&spec.content, &spec.node) {
            (Some(content), _) => {
                let (bytes, data_media) = decode_content(content, index, limits.max_bytes_each)?;
                let filename = spec
                    .filename
                    .clone()
                    .expect("validated: inline content requires a filename");
                let content_type = spec
                    .content_type
                    .clone()
                    .or(data_media)
                    .unwrap_or_else(|| guess_content_type(&filename));
                (bytes, filename, content_type)
            }
            (None, Some(node_path)) => self.resolve_node_ref(&spec, node_path, index).await?,
            (None, None) => unreachable!("validated: exactly one source"),
        };

        // A fetch guard, NOT a duplicate of the validation rule: by the time
        // `validate_attachments` runs the bytes are already resident, so the
        // limit has to be applied here too to keep an oversized blob from
        // being read into memory at all.
        if bytes.len() > limits.max_bytes_each {
            return Err(invalid(format!(
                "attachments[{index}] `{filename}` is {} bytes (max {} each)",
                bytes.len(),
                limits.max_bytes_each
            )));
        }

        Ok(ResolvedAttachment {
            filename,
            content_type,
            bytes,
            content_id: spec.content_id.clone(),
        })
    }

    /// Read a node's resource property and fetch its bytes.
    ///
    /// Goes through the ordinary node API so row-level security applies, and
    /// insists on a `Resource` property: without that check this would be a
    /// way to mail out any property of any node as a text file.
    async fn resolve_node_ref(
        &self,
        spec: &AttachmentSpec,
        node_path: &str,
        index: usize,
    ) -> Result<(Vec<u8>, String, String)> {
        let workspace = spec.workspace.as_deref().unwrap_or("").trim();
        if workspace.is_empty() {
            return Err(invalid(format!(
                "attachments[{index}] references node `{node_path}` but names no workspace"
            )));
        }
        let property = spec
            .property
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .unwrap_or(DEFAULT_RESOURCE_PROPERTY);

        let node = self
            .impl_node_get(workspace, node_path)
            .await?
            .ok_or_else(|| {
                invalid(format!(
                    "attachments[{index}]: no node at `{workspace}:{node_path}`"
                ))
            })?;

        let resource = node
            .get("properties")
            .and_then(|p| p.get(property))
            .ok_or_else(|| {
                invalid(format!(
                    "attachments[{index}]: node `{workspace}:{node_path}` has no property `{property}`"
                ))
            })?;

        let storage_key = resource
            .get("metadata")
            .and_then(|m| m.get("storage_key"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                invalid(format!(
                    "attachments[{index}]: `{workspace}:{node_path}` property `{property}` is not a file \
                     (no resource metadata); only resource properties can be attached"
                ))
            })?;

        let bytes = self.impl_resource_get_bytes(storage_key).await?;

        // The stored name was not authored to be a MIME header, so it is
        // basenamed and sanitized rather than rejected — unlike a filename the
        // caller wrote for this send.
        let filename = match spec.filename.clone() {
            Some(f) => f,
            None => {
                let stored = resource
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("");
                let cleaned: String = stored
                    .chars()
                    .filter(|c| (*c as u32) >= 0x20 && *c as u32 != 0x7f && *c != ':')
                    .take(MAX_FILENAME_LEN)
                    .collect();
                let cleaned = cleaned.trim().to_string();
                if cleaned.is_empty() {
                    format!("attachment-{}", index + 1)
                } else {
                    cleaned
                }
            }
        };

        let content_type = match spec.content_type.clone() {
            Some(ct) => ct,
            None => match resource.get("mime_type").and_then(Value::as_str) {
                // Stored metadata is caller-influenced too (`addResource`
                // takes a mimeType), so it goes through the same sanitiser.
                Some(mt) if !mt.trim().is_empty() => {
                    check_content_type(mt, index).unwrap_or_else(|_| guess_content_type(&filename))
                }
                _ => guess_content_type(&filename),
            },
        };

        Ok((bytes, filename, content_type))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs(v: Value) -> Result<Vec<AttachmentSpec>> {
        let mut m = serde_json::json!({ "to": "a@b.c", "attachments": v });
        take_attachment_specs(&mut m)
    }

    #[test]
    fn an_absent_attachments_key_is_not_an_error() {
        let mut m = serde_json::json!({ "to": "a@b.c" });
        assert!(take_attachment_specs(&mut m).unwrap().is_empty());
    }

    /// The key must be REMOVED, or it reaches `EmailMessage` deserialization.
    #[test]
    fn the_key_is_lifted_out_of_the_message() {
        let mut m = serde_json::json!({
            "to": "a@b.c",
            "attachments": [{ "content": "aGk=", "filename": "a.txt" }]
        });
        take_attachment_specs(&mut m).unwrap();
        assert!(
            m.get("attachments").is_none(),
            "attachments must be removed"
        );
    }

    #[test]
    fn exactly_one_source_is_required() {
        let both = specs(serde_json::json!([{
            "content": "aGk=", "filename": "a.txt", "node": "/x", "workspace": "assets"
        }]));
        assert!(both.unwrap_err().to_string().contains("exactly one source"));

        let neither = specs(serde_json::json!([{ "filename": "a.txt" }]));
        assert!(neither.unwrap_err().to_string().contains("no source"));
    }

    #[test]
    fn inline_content_requires_a_filename() {
        let err = specs(serde_json::json!([{ "content": "aGk=" }])).unwrap_err();
        assert!(err.to_string().contains("needs a `filename`"), "{err}");
    }

    #[test]
    fn a_filename_may_not_split_headers_or_carry_a_path() {
        for bad in [
            "a\r\nBcc: attacker@evil.test",
            "../../etc/passwd",
            "dir/file.pdf",
            "back\\slash.pdf",
            "c:file.pdf",
        ] {
            let err = specs(serde_json::json!([{ "content": "aGk=", "filename": bad }]))
                .expect_err(&format!("`{bad}` must be refused"));
            let msg = err.to_string();
            assert!(
                msg.contains("control characters") || msg.contains("path separator"),
                "`{bad}` gave: {msg}"
            );
        }
    }

    /// A caller-set boundary would let one leaf declare a whole MIME subtree.
    #[test]
    fn a_multipart_content_type_is_refused() {
        let err = specs(serde_json::json!([{
            "content": "aGk=", "filename": "a.txt",
            "content_type": "multipart/mixed; boundary=xyz"
        }]))
        .unwrap_err();
        assert!(err.to_string().contains("smuggle"), "{err}");
    }

    #[test]
    fn content_type_parameters_are_stripped() {
        let s = specs(serde_json::json!([{
            "content": "aGk=", "filename": "a.txt",
            "content_type": "TEXT/Plain; charset=utf-8"
        }]))
        .unwrap();
        assert_eq!(s[0].content_type.as_deref(), Some("text/plain"));
    }

    /// Angle brackets must come off, or lettre doubles them and every client
    /// stops matching `cid:`.
    #[test]
    fn a_bracketed_content_id_is_normalised() {
        let s = specs(serde_json::json!([{
            "content": "aGk=", "filename": "a.png", "content_id": "<logo@x>"
        }]))
        .unwrap();
        assert_eq!(s[0].content_id.as_deref(), Some("logo@x"));
    }

    #[test]
    fn a_hostile_content_id_is_refused() {
        for bad in ["a b", "a\r\nX: y", "a>b<c"] {
            assert!(
                specs(serde_json::json!([{
                    "content": "aGk=", "filename": "a.png", "content_id": bad
                }]))
                .is_err(),
                "`{bad}` must be refused"
            );
        }
    }

    #[test]
    fn a_data_url_yields_bytes_and_its_media_type() {
        let (bytes, mt) = decode_content("data:application/pdf;base64,aGVsbG8=", 0, 1024).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(mt.as_deref(), Some("application/pdf"));
    }

    #[test]
    fn a_non_base64_data_url_is_refused() {
        assert!(decode_content("data:text/plain,hello", 0, 1024).is_err());
    }

    #[test]
    fn whitespace_wrapped_base64_decodes() {
        let (bytes, _) = decode_content("aGVs\nbG8=\n", 0, 1024).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn url_safe_base64_is_refused_rather_than_guessed() {
        let err = decode_content("a-b_c", 0, 1024).unwrap_err();
        assert!(err.to_string().contains("URL-safe"), "{err}");
    }

    /// The length check must come BEFORE the allocation.
    #[test]
    fn an_oversized_payload_is_refused_without_decoding() {
        let huge = "A".repeat(10_000);
        let err = decode_content(&huge, 0, 16).unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
    }

    #[test]
    fn a_decode_error_never_echoes_the_payload() {
        let err = decode_content("!!!!not base64!!!!", 0, 1024).unwrap_err();
        assert!(!err.to_string().contains("not base64!!!!"), "{err}");
        assert!(err.to_string().contains("not valid base64"), "{err}");
    }

    #[test]
    fn content_type_is_guessed_from_the_filename() {
        assert_eq!(guess_content_type("ticket.pdf"), "application/pdf");
        assert_eq!(guess_content_type("logo.png"), "image/png");
        assert_eq!(guess_content_type("mystery"), "application/octet-stream");
    }
}
