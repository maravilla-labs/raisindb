//! Streaming a node's bytes to a URL the adapter minted.
//!
//! # Why the engine does this and not the adapter
//!
//! An adapter runs in QuickJS with a memory budget measured in tens of
//! megabytes, and everything it receives crosses that boundary as a JS string.
//! A 200 MB video cannot be handed to it, and a provider's resumable-upload
//! protocol wants ranged PUTs of raw bytes — so the adapter negotiates the
//! session (an ordinary JSON call it is well suited to) and hands back the URL,
//! and the transfer happens here where bytes are bytes.
//!
//! This is the exact mirror of [`super::super::content::fetch_url`], which
//! exists for the same reason in the opposite direction, and it reuses that
//! module's guarantees: the same operator-owned [`EgressPolicy`], the same
//! refusal to follow redirects, the same "used immediately, never stored".
//!
//! # Why the provider's answer goes back to the adapter
//!
//! The final chunk's response carries the created object — for Graph, the whole
//! driveItem with its `id` and `@odata.etag`. Parsing that here would put
//! provider-shaped knowledge in the engine, which is the one thing the adapter
//! boundary exists to prevent. So the engine reports the transfer and the
//! adapter's `finalize_upload` says what was created.

use raisin_error::{Error, Result};
use raisin_mcp_protocol::client::{installed_egress_policy, EgressPolicy};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use url::Url;

/// Wall-clock ceiling for one upload, chunks included. Generous next to the
/// download's 120s because the upstream leg of a home connection is the slow
/// one, and a large file legitimately takes minutes.
const UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Default bytes per PUT when the adapter names no size.
///
/// Providers that use ranged uploads generally require a multiple of 320 KiB;
/// 10 MiB is a common recommendation and a safe default for one that does not
/// care. An adapter with a rule of its own sends `chunk_size` and wins.
const DEFAULT_CHUNK_BYTES: usize = 10 * 320 * 1024;

/// What the adapter asked the engine to do with the bytes.
pub(crate) struct UploadRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub chunk_size: usize,
}

impl UploadRequest {
    /// Parse an adapter's `{ upload: { … } }` answer, or `None` when it did not
    /// ask for one.
    pub(crate) fn from_adapter_value(result: &Value) -> Option<Self> {
        let upload = result.get("upload")?.as_object()?;
        let url = upload.get("url")?.as_str()?.to_string();
        if url.is_empty() {
            return None;
        }
        let method = upload
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("PUT")
            .to_uppercase();
        let headers = upload
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let chunk_size = upload
            .get("chunk_size")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_CHUNK_BYTES);
        Some(Self {
            url,
            method,
            headers,
            chunk_size,
        })
    }
}

/// The provider's answer to the last chunk, handed back to the adapter.
#[derive(Debug)]
pub(crate) struct UploadOutcome {
    pub status: u16,
    pub body: Value,
}

/// Send `bytes` to the adapter-supplied URL, in ranged chunks.
///
/// The URL is treated as hostile input for the same reason `fetch_url` is: it
/// arrived from an adapter, and dialling it from inside the cluster is the
/// textbook SSRF shape. It is checked through the operator's own egress policy,
/// including the post-DNS re-resolution guard, before a single byte moves.
pub(crate) async fn upload_bytes(
    request: &UploadRequest,
    bytes: Vec<u8>,
    mime_type: &str,
) -> Result<UploadOutcome> {
    let url = Url::parse(&request.url).map_err(|e| {
        Error::Validation(format!("adapter returned an unparseable upload url: {e}"))
    })?;
    let policy: EgressPolicy = installed_egress_policy();
    policy
        .guard(&url)
        .await
        .map_err(|e| Error::Validation(format!("upload url refused: {e}")))?;

    let client = reqwest::Client::builder()
        .timeout(UPLOAD_TIMEOUT)
        // A redirect is a second destination the policy never saw, and a
        // redirected upload would also silently restart the byte range.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Error::Backend(format!("upload client build failed: {e}")))?;

    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|_| Error::Validation(format!("invalid upload method '{}'", request.method)))?;

    let mut extra = HeaderMap::new();
    for (k, v) in &request.headers {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) else {
            return Err(Error::Validation(format!(
                "adapter supplied an unusable upload header '{k}'"
            )));
        };
        extra.insert(name, value);
    }

    let total = bytes.len();
    // A zero-byte file is legitimate (a touched placeholder), and chunking
    // arithmetic over an empty range produces no requests at all — so it is
    // sent as one empty body rather than skipped, which would report success
    // for an object never created.
    let mut sent = 0usize;
    let mut last: Option<UploadOutcome> = None;
    loop {
        let end = std::cmp::min(sent + request.chunk_size, total);
        let chunk = bytes[sent..end].to_vec();
        let is_last = end == total;

        let mut req = client
            .request(method.clone(), url.clone())
            .headers(extra.clone())
            .header(reqwest::header::CONTENT_TYPE, mime_type)
            .header(reqwest::header::CONTENT_LENGTH, chunk.len());
        // Ranged only when the transfer is actually chunked. A provider that
        // takes the whole body in one request may reject a Content-Range it
        // never asked for.
        if total > request.chunk_size {
            req = req.header(
                "Content-Range",
                format!("bytes {}-{}/{}", sent, end.saturating_sub(1), total),
            );
        }

        let resp = req
            .body(chunk)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("upload chunk failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            let hint = if status.as_u16() == 401 || status.as_u16() == 403 {
                " (an upload url is short-lived and pre-authenticated — the adapter must mint one per push, not persist it)"
            } else {
                ""
            };
            return Err(Error::Backend(format!(
                "upload returned HTTP {status}{hint}: {}",
                detail.chars().take(400).collect::<String>()
            )));
        }

        if is_last {
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            last = Some(UploadOutcome {
                status: status.as_u16(),
                body,
            });
            break;
        }
        sent = end;
    }

    last.ok_or_else(|| Error::Backend("upload produced no final response".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The ordinary answer — an adapter that finished the write itself — must
    /// not be mistaken for an upload request.
    #[test]
    fn an_answer_without_an_upload_asks_for_no_transfer() {
        let done = json!({ "external_id": "01ABC", "etag": "\"1\"" });
        assert!(UploadRequest::from_adapter_value(&done).is_none());
    }

    /// An empty url is a bug in the adapter, and treating it as a transfer
    /// would mean dialling `""` and reporting a network error for a
    /// configuration one.
    #[test]
    fn an_empty_upload_url_is_not_a_transfer() {
        let bad = json!({ "upload": { "url": "" } });
        assert!(UploadRequest::from_adapter_value(&bad).is_none());
    }

    #[test]
    fn an_upload_defaults_to_a_chunked_put() {
        let ask = json!({ "upload": { "url": "https://example.test/u" } });
        let req = UploadRequest::from_adapter_value(&ask).expect("upload");
        assert_eq!(req.method, "PUT");
        assert_eq!(req.chunk_size, DEFAULT_CHUNK_BYTES);
        assert!(req.headers.is_empty());
    }

    #[test]
    fn the_adapter_may_state_method_headers_and_chunk_size() {
        let ask = json!({
            "upload": {
                "url": "https://example.test/u",
                "method": "post",
                "headers": { "x-goog-resumable": "start" },
                "chunk_size": 655360,
            }
        });
        let req = UploadRequest::from_adapter_value(&ask).expect("upload");
        assert_eq!(req.method, "POST");
        assert_eq!(req.chunk_size, 655360);
        assert_eq!(
            req.headers,
            vec![("x-goog-resumable".to_string(), "start".to_string())]
        );
    }

    /// A zero `chunk_size` would divide the transfer into infinitely many
    /// empty requests; it falls back rather than hanging.
    #[test]
    fn a_zero_chunk_size_falls_back_to_the_default() {
        let ask = json!({ "upload": { "url": "https://example.test/u", "chunk_size": 0 } });
        let req = UploadRequest::from_adapter_value(&ask).expect("upload");
        assert_eq!(req.chunk_size, DEFAULT_CHUNK_BYTES);
    }

    /// The URL arrives from an adapter, so it is hostile input: the egress
    /// policy must refuse a loopback destination before any byte moves.
    #[tokio::test]
    async fn a_loopback_upload_url_is_refused() {
        let req = UploadRequest {
            url: "http://127.0.0.1:9/upload".to_string(),
            method: "PUT".to_string(),
            headers: vec![],
            chunk_size: DEFAULT_CHUNK_BYTES,
        };
        let err = upload_bytes(&req, vec![1, 2, 3], "application/octet-stream")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("refused"),
            "expected an egress refusal, got {err}"
        );
    }
}
