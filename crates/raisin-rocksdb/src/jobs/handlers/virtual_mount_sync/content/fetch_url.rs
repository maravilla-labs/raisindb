//! Fetching content the adapter could only point at, not carry.
//!
//! # Why this exists
//!
//! An adapter runs in QuickJS, and the host's `raisin.http.fetch` decodes every
//! response as TEXT (`response.text()`). A JS string cannot hold arbitrary
//! bytes, so a PDF or an image round-tripped through one comes back corrupted
//! with no error anywhere — which is why `get_content` requires `content_base64`
//! for binary and why the MS Graph drive branch refused to serve file content at
//! all rather than serve it wrong.
//!
//! Some providers hand out the bytes as a **pre-authenticated, short-lived
//! URL** instead of an inline payload — Microsoft Graph's
//! `@microsoft.graph.downloadUrl` is the canonical case, valid for roughly an
//! hour. Persisting such a URL on the node is a trap: it is dead within the
//! hour while still looking like a durable link, and nothing re-reads the item
//! unless its etag changes.
//!
//! So the adapter answers `{ fetch_url }` with a URL it minted *just now*, and
//! the engine does the download in Rust, where bytes are bytes. The URL is used
//! immediately and never stored.
//!
//! # Why this is guarded
//!
//! The engine is dialling a URL that arrived from an adapter — the textbook SSRF
//! shape: `http://169.254.169.254/…` reaches cloud instance metadata and
//! `http://10.0.0.5/` reaches the private network the database sits in. It is
//! checked through the SAME operator-owned [`EgressPolicy`] the MCP client uses
//! (`[mcp_client]` config), including its re-resolution guard against DNS
//! rebinding — one policy, one implementation, rather than a second copy that
//! drifts.

use raisin_error::{Error, Result};
use raisin_mcp_protocol::client::{installed_egress_policy, EgressPolicy};
use url::Url;

/// Ceiling on one fetched object. Mirrors the inline path's ceiling, and is
/// enforced BEFORE the body is buffered wherever the server declares a length,
/// so a hostile `Content-Length` cannot make us allocate first and check after.
pub(crate) const MAX_FETCH_BYTES: u64 = 64 * 1024 * 1024;

/// Wall-clock ceiling for one content download.
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Fetch the bytes an adapter pointed at, or explain why we would not.
///
/// `mime` is the adapter's declared type; the response's own `Content-Type`
/// only fills in when the adapter did not state one, because the adapter knows
/// what the ITEM is and a download host often answers
/// `application/octet-stream` for everything.
pub(crate) async fn fetch_content_url(
    raw_url: &str,
    declared_mime: Option<String>,
) -> Result<(Vec<u8>, String)> {
    let url = Url::parse(raw_url).map_err(|e| {
        Error::Validation(format!(
            "get_content returned an unparseable fetch_url: {e}"
        ))
    })?;

    // Scheme, host, allow-list, and the resolved addresses. `guard` performs the
    // second (post-DNS) check, which is what a save-time-only validation misses.
    let policy: EgressPolicy = installed_egress_policy();
    policy
        .guard(&url)
        .await
        .map_err(|e| Error::Validation(format!("get_content fetch_url refused: {e}")))?;

    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // A redirect is a second destination the policy never saw. Following one
        // would let a public URL bounce us to 127.0.0.1 and defeat the guard
        // above, so redirects are refused rather than re-validated: a provider's
        // download host answers 200 directly.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Error::Backend(format!("content fetch client build failed: {e}")))?;

    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| Error::Backend(format!("content fetch failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        // A 403 here is the ordinary expiry of a pre-authenticated link that
        // was minted too long ago — say so, because "403" alone sends an
        // operator looking at permissions.
        return Err(Error::Backend(format!(
            "content fetch returned HTTP {status}{}",
            if status.as_u16() == 403 || status.as_u16() == 401 {
                " (a pre-authenticated download link is short-lived — the adapter must mint one per fetch, not persist it)"
            } else {
                ""
            }
        )));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_FETCH_BYTES {
            return Err(Error::Validation(format!(
                "content fetch declared {len} bytes, over the {MAX_FETCH_BYTES}-byte ceiling"
            )));
        }
    }

    let mime = declared_mime
        .or_else(|| {
            resp.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                // Strip any `; charset=…` — the stored asset wants the type.
                .map(|s| s.split(';').next().unwrap_or(s).trim().to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Backend(format!("content fetch body read failed: {e}")))?;

    // Checked again after the fact: `Content-Length` is optional and a chunked
    // response can exceed a limit it never declared.
    if bytes.len() as u64 > MAX_FETCH_BYTES {
        return Err(Error::Validation(format!(
            "content fetch returned {} bytes, over the {MAX_FETCH_BYTES}-byte ceiling",
            bytes.len()
        )));
    }

    Ok((bytes.to_vec(), mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard must refuse the addresses that make this an SSRF vector, and
    /// must refuse a non-HTTPS scheme, BEFORE any request is made.
    #[tokio::test]
    async fn a_fetch_url_pointing_inward_is_refused() {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "https://169.254.169.254/latest/meta-data/",
            "https://127.0.0.1/secret",
            "https://[::1]/secret",
            "http://example.com/plain-http",
            "file:///etc/passwd",
            "not a url",
        ] {
            let err = fetch_content_url(url, None).await.unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("refused") || msg.contains("unparseable"),
                "{url} must be refused before any request; got: {msg}"
            );
        }
    }
}
