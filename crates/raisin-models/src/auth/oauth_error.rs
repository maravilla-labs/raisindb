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

//! Diagnostic extraction from an OAuth token-endpoint error body.
//!
//! Lives here, rather than beside either caller, because there are exactly two
//! places that POST to a provider's token endpoint and they are in different
//! crates: the interactive authorization-code exchange
//! (`raisin-transport-http`, `handlers/integrations/oauth_callback.rs`) and the
//! background refresh grant (`raisin-rocksdb`,
//! `jobs/handlers/integration_token_refresh.rs`). Both must redact identically —
//! a copy in each is precisely the mirrored-path drift that turns one hardened
//! site and one leaky one into a credential disclosure.

use serde_json::Value;

/// Longest `error` code echoed. Real codes are short (`invalid_client`).
const MAX_CODE_CHARS: usize = 64;
/// Longest `error_description` echoed. Microsoft's `AADSTS…` text plus a
/// correlation id fits comfortably; a hostile provider cannot flood a log line.
const MAX_DESC_CHARS: usize = 400;

/// Extract the RFC 6749 §5.2 `error` / `error_description` pair from a token
/// endpoint's error response, clamped, as `(code, description)`.
///
/// Those two fields are defined by the spec as diagnostic text describing why a
/// request was refused; they carry the provider's own failure code and never
/// credential material. **Only** those two fields are read — the remainder of
/// the body is discarded rather than echoed, so a provider that returns
/// something unexpected (or a token) alongside its error cannot leak it into a
/// log line, an API response, or a redirect URL.
///
/// Either half is empty when the body is not JSON or omits that field, leaving
/// the caller to fall back to the bare HTTP status.
///
/// This is what makes a provider `401` actionable: Microsoft answers a wrong
/// client secret, an unsent client secret, and a redirect URI registered under
/// the wrong application platform all with `invalid_client`, and only the
/// `AADSTS…` code in the description tells them apart.
pub fn oauth_error_parts(body: &str) -> (String, String) {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return (String::new(), String::new());
    };
    let field = |key: &str, max: usize| -> String {
        v.get(key)
            .and_then(|f| f.as_str())
            .map(|s| s.chars().take(max).collect::<String>())
            .unwrap_or_default()
    };
    (
        field("error", MAX_CODE_CHARS),
        field("error_description", MAX_DESC_CHARS),
    )
}

/// [`oauth_error_parts`] joined into one human-readable string for a log line or
/// an error message. Empty when neither field was present.
pub fn oauth_error_detail(body: &str) -> String {
    let (code, desc) = oauth_error_parts(body);
    match (code.is_empty(), desc.is_empty()) {
        (true, true) => String::new(),
        (false, true) => code,
        (true, false) => desc,
        (false, false) => format!("{code}: {desc}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_both_fields() {
        let body = r#"{"error":"invalid_client","error_description":"AADSTS7000215: Invalid client secret provided."}"#;
        assert_eq!(
            oauth_error_detail(body),
            "invalid_client: AADSTS7000215: Invalid client secret provided."
        );
    }

    #[test]
    fn each_field_alone() {
        assert_eq!(
            oauth_error_detail(r#"{"error":"invalid_grant"}"#),
            "invalid_grant"
        );
        assert_eq!(
            oauth_error_detail(r#"{"error_description":"nope"}"#),
            "nope"
        );
    }

    /// The caller falls back to the bare status on an empty return, so these
    /// must not produce a misleading partial string.
    #[test]
    fn empty_when_unusable() {
        assert_eq!(oauth_error_detail(""), "");
        assert_eq!(oauth_error_detail("<html>502</html>"), "");
        assert_eq!(oauth_error_detail(r#"{"foo":"bar"}"#), "");
        assert_eq!(oauth_error_detail(r#"{"error":123}"#), "");
    }

    /// Anything outside the two spec-defined diagnostic fields is dropped —
    /// this is the redaction guarantee, not an incidental detail.
    #[test]
    fn ignores_every_other_field() {
        let body = r#"{"error":"invalid_client","access_token":"secret-token","refresh_token":"secret-refresh","client_secret":"hunter2"}"#;
        let out = oauth_error_detail(body);
        assert_eq!(out, "invalid_client");
        assert!(!out.contains("secret"));
        assert!(!out.contains("hunter2"));
    }

    #[test]
    fn clamps_a_flooding_provider() {
        let body = format!(
            r#"{{"error":"{}","error_description":"{}"}}"#,
            "e".repeat(500),
            "d".repeat(5000)
        );
        let out = oauth_error_detail(&body);
        // code + ": " + desc
        assert_eq!(out.chars().count(), MAX_CODE_CHARS + 2 + MAX_DESC_CHARS);
    }

    /// Clamping is char-based, so a multi-byte description can never be split
    /// mid-codepoint (which would panic on a byte-slice).
    #[test]
    fn clamp_never_splits_utf8() {
        let body = format!(r#"{{"error_description":"{}"}}"#, "é".repeat(5000));
        assert_eq!(oauth_error_detail(&body).chars().count(), MAX_DESC_CHARS);
    }
}
