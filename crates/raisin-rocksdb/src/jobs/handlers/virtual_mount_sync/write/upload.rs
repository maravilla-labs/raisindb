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
//!
//! # Why nothing here knows a provider's protocol
//!
//! This module was written against Microsoft Graph and grew three of its
//! assumptions, each of which broke a different provider (audit, Aug 2026):
//!
//! * **"non-2xx means the transfer failed."** Google Drive's resumable
//!   protocol answers `308 Resume Incomplete` to every non-final chunk, so a
//!   Drive upload failed on chunk one. The adapter now declares
//!   `continue_statuses`.
//! * **"a chunk is a multiple of 320 KiB."** That is Microsoft's unit; Drive's
//!   is 256 KiB and S3 has no ranged protocol at all. The engine sends
//!   `chunk_size` VERBATIM and rounds nothing — the adapter owns its
//!   provider's rule.
//! * **"the answer is a JSON body."** S3's `PutObject` answers with an EMPTY
//!   body and the ETag in a header. `finalize_upload` now receives the
//!   response headers alongside the body.
//!
//! The rule the three share: the engine moves bytes and reports what came
//! back; every judgement about what a provider's answer MEANS belongs to the
//! adapter.

use raisin_error::{Error, Result};
use raisin_mcp_protocol::client::{installed_egress_policy, EgressPolicy};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::collections::BTreeMap;
use url::Url;

/// Wall-clock ceiling for one upload, chunks included. Generous next to the
/// download's 120s because the upstream leg of a home connection is the slow
/// one, and a large file legitimately takes minutes.
const UPLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Default bytes per PUT when the adapter names no size.
///
/// 10 MiB, which is a whole multiple of BOTH 320 KiB (Graph's unit) and
/// 256 KiB (Drive's), so the fallback is a legal chunk under either rule.
///
/// This was `10 * 320 * 1024` — 3.125 MiB, despite a comment claiming 10 MiB —
/// and 3.125 MiB is NOT a multiple of 256 KiB, so a Drive adapter that named
/// no `chunk_size` would have had every non-final chunk rejected outright.
/// The test below pins the arithmetic so the next edit cannot re-break it.
///
/// It is a FALLBACK, not a policy: the size a provider requires is provider
/// knowledge, so the adapter owns it and sends `chunk_size`.
const DEFAULT_CHUNK_BYTES: usize = 10 * 1024 * 1024;

/// Statuses that mean "chunk accepted, keep sending" when the adapter names
/// none.
///
/// A 2xx on a non-final chunk is already accepted by the success rule, so the
/// only status the default must add is Google Drive's `308 Resume Incomplete`
/// — the answer its resumable protocol gives to EVERY non-final chunk, and the
/// one that made a Drive upload fail on chunk one. An adapter whose provider
/// signals continuation some other way sends `continue_statuses` and replaces
/// this list outright; an explicitly empty list means "nothing but 2xx", which
/// is a legitimate thing to ask for.
const DEFAULT_CONTINUE_STATUSES: &[u16] = &[308];

/// What the adapter asked the engine to do with the bytes.
pub(crate) struct UploadRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    /// Sent verbatim. The engine deliberately does not round this to any
    /// boundary: Graph wants a multiple of 320 KiB and Drive a multiple of
    /// 256 KiB, and an engine that "helpfully" rounded would be wrong for one
    /// of them. The adapter knows its provider's rule; the engine does not.
    pub chunk_size: usize,
    /// Non-2xx statuses that mean "chunk accepted, keep going".
    pub continue_statuses: Vec<u16>,
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
        // Absent means "use the default"; present-but-empty means "2xx only",
        // which is a different and legitimate answer — so the fallback hangs
        // off the key's absence, not off the list being empty.
        let continue_statuses = upload
            .get("continue_statuses")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_u64())
                    // Anything outside the HTTP status range cannot ever be
                    // compared against a real response, so it is noise rather
                    // than a rule; dropping it keeps a typo from silently
                    // widening what counts as success.
                    .filter(|n| (100..=599).contains(n))
                    .map(|n| n as u16)
                    .collect()
            })
            .unwrap_or_else(|| DEFAULT_CONTINUE_STATUSES.to_vec());
        Some(Self {
            url,
            method,
            headers,
            chunk_size,
            continue_statuses,
        })
    }
}

/// The provider's answer to the last chunk, handed back to the adapter.
#[derive(Debug)]
pub(crate) struct UploadOutcome {
    pub status: u16,
    pub body: Value,
    /// Response headers, keys lowercased.
    ///
    /// A JSON body is not the only place a provider puts the result: S3's
    /// `PutObject` answers with an EMPTY body and the ETag in a header, so an
    /// outcome of status + body alone gave the adapter nothing to finalize
    /// with. Lowercased because HTTP header names are case-insensitive and an
    /// adapter should not have to guess whether this provider wrote `ETag`,
    /// `Etag` or `etag`.
    pub headers: BTreeMap<String, String>,
}

/// What one chunk's status code means for the transfer.
#[derive(Debug, PartialEq, Eq)]
enum ChunkVerdict {
    /// The transfer is complete; hand the response to the adapter.
    Done,
    /// The provider took this chunk and wants the next one.
    Continue,
    /// Stop and report.
    Failed,
}

/// Judge one chunk's status.
///
/// Split out from the transfer loop because it is the whole of the
/// provider-neutrality rule and the only part of it that can be tested without
/// a network: a non-final chunk may be answered by a success status OR by one
/// the adapter declared as continuation (Drive's `308`), while the FINAL chunk
/// must be a success — a 308 there says the provider does not consider the
/// object written, and accepting it would report a truncated upload as done.
fn classify_chunk(status: u16, is_last: bool, continue_statuses: &[u16]) -> ChunkVerdict {
    let success = (200..300).contains(&status);
    if is_last {
        return if success {
            ChunkVerdict::Done
        } else {
            ChunkVerdict::Failed
        };
    }
    if success || continue_statuses.contains(&status) {
        ChunkVerdict::Continue
    } else {
        ChunkVerdict::Failed
    }
}

/// Collect a response's headers into the plain lowercase map the adapter sees.
///
/// A repeated header is joined with `", "` rather than dropped, which is how
/// HTTP defines the equivalent single-field form.
fn collect_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers.iter() {
        // A header whose bytes are not UTF-8 cannot cross the QuickJS string
        // boundary at all, so it is skipped rather than lossily mangled.
        let Ok(value) = value.to_str() else { continue };
        out.entry(name.as_str().to_ascii_lowercase())
            .and_modify(|existing| {
                existing.push_str(", ");
                existing.push_str(value);
            })
            .or_insert_with(|| value.to_string());
    }
    out
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

        let status = resp.status().as_u16();
        match classify_chunk(status, is_last, &request.continue_statuses) {
            ChunkVerdict::Failed => {
                let detail = resp.text().await.unwrap_or_default();
                let hint = if status == 401 || status == 403 {
                    " (an upload url is short-lived and pre-authenticated — the adapter must mint one per push, not persist it)"
                } else if !is_last && !request.continue_statuses.is_empty() {
                    " (the adapter declared continue_statuses, and this is not one of them)"
                } else if !is_last {
                    " (a provider whose resumable protocol answers a non-final chunk with a non-2xx status — Drive's 308 — must declare it in continue_statuses)"
                } else {
                    ""
                };
                return Err(Error::Backend(format!(
                    "upload returned HTTP {status}{hint}: {}",
                    detail.chars().take(400).collect::<String>()
                )));
            }
            ChunkVerdict::Done => {
                // Headers first: reading the body consumes the response, and
                // for S3 the headers are the ONLY place the result lives.
                let headers = collect_headers(resp.headers());
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                last = Some(UploadOutcome {
                    status,
                    body,
                    headers,
                });
                break;
            }
            ChunkVerdict::Continue => sent = end,
        }
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
            continue_statuses: DEFAULT_CONTINUE_STATUSES.to_vec(),
        };
        let err = upload_bytes(&req, vec![1, 2, 3], "application/octet-stream")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("refused"),
            "expected an egress refusal, got {err}"
        );
    }

    // --- non-2xx is not always failure ------------------------------------

    /// The bug this replaced: every non-success status failed the transfer, so
    /// Google Drive — which answers `308 Resume Incomplete` to EVERY non-final
    /// chunk — could never upload a file larger than one chunk.
    #[test]
    fn a_declared_continue_status_keeps_a_chunked_upload_going() {
        assert_eq!(
            classify_chunk(308, false, &[308]),
            ChunkVerdict::Continue,
            "308 Resume Incomplete is Drive saying 'send the next chunk'"
        );
    }

    /// 308 is only a continuation, never a completion: accepting it on the
    /// last chunk would report a truncated object as successfully written.
    #[test]
    fn a_continue_status_on_the_final_chunk_still_fails() {
        assert_eq!(classify_chunk(308, true, &[308]), ChunkVerdict::Failed);
    }

    /// A status in neither set fails loudly wherever it lands — the whole
    /// point of letting the adapter widen the set is that it stays a set.
    #[test]
    fn a_status_outside_both_sets_fails_on_any_chunk() {
        assert_eq!(classify_chunk(500, false, &[308]), ChunkVerdict::Failed);
        assert_eq!(classify_chunk(500, true, &[308]), ChunkVerdict::Failed);
        assert_eq!(classify_chunk(403, false, &[308]), ChunkVerdict::Failed);
    }

    /// A 2xx on a non-final chunk is Graph's answer and needs no declaration;
    /// the continue list only ever ADDS to the success rule.
    #[test]
    fn a_success_status_continues_without_being_declared() {
        assert_eq!(classify_chunk(202, false, &[]), ChunkVerdict::Continue);
        assert_eq!(classify_chunk(200, true, &[]), ChunkVerdict::Done);
    }

    /// The default exists so a Drive adapter that forgets the key still works;
    /// Graph is unaffected because its statuses were already 2xx.
    #[test]
    fn an_adapter_that_names_no_continue_statuses_still_resumes_a_308() {
        let ask = json!({ "upload": { "url": "https://example.test/u" } });
        let req = UploadRequest::from_adapter_value(&ask).expect("upload");
        assert_eq!(req.continue_statuses, vec![308]);
        assert_eq!(
            classify_chunk(308, false, &req.continue_statuses),
            ChunkVerdict::Continue
        );
    }

    /// An adapter's list REPLACES the default rather than extending it, so a
    /// provider that uses a different signal is not also silently resuming on
    /// a 308 it never meant.
    #[test]
    fn a_declared_list_replaces_the_default() {
        let ask = json!({
            "upload": { "url": "https://example.test/u", "continue_statuses": [100, 308] }
        });
        let req = UploadRequest::from_adapter_value(&ask).expect("upload");
        assert_eq!(req.continue_statuses, vec![100, 308]);

        let strict = json!({
            "upload": { "url": "https://example.test/u", "continue_statuses": [] }
        });
        let req = UploadRequest::from_adapter_value(&strict).expect("upload");
        assert!(
            req.continue_statuses.is_empty(),
            "an explicit empty list means 2xx only, not 'use the default'"
        );
        assert_eq!(
            classify_chunk(308, false, &req.continue_statuses),
            ChunkVerdict::Failed
        );
    }

    /// A value that is not a status code can never match a response, so it is
    /// dropped rather than widening the set by accident.
    #[test]
    fn nonsense_continue_statuses_are_dropped() {
        let ask = json!({
            "upload": { "url": "https://example.test/u", "continue_statuses": [0, 308, 9000, "308"] }
        });
        let req = UploadRequest::from_adapter_value(&ask).expect("upload");
        assert_eq!(req.continue_statuses, vec![308]);
    }

    // --- chunk size is the adapter's, verbatim ----------------------------

    /// The engine must not round to Microsoft's 320 KiB (or anyone's): Drive
    /// requires a multiple of 256 KiB, and a rounded chunk is rejected by
    /// whichever provider it was not rounded for.
    #[test]
    fn an_adapter_chunk_size_is_honoured_exactly() {
        for size in [256 * 1024usize, 8 * 256 * 1024, 320 * 1024, 262_145, 1] {
            let ask = json!({
                "upload": { "url": "https://example.test/u", "chunk_size": size }
            });
            let req = UploadRequest::from_adapter_value(&ask).expect("upload");
            assert_eq!(
                req.chunk_size, size,
                "chunk_size must reach the wire unrounded"
            );
        }
    }

    /// The fallback has to be legal under BOTH providers' rules, because it is
    /// used precisely when nobody said which provider this is.
    #[test]
    fn the_default_chunk_is_a_multiple_of_both_providers_units() {
        assert_eq!(DEFAULT_CHUNK_BYTES % (320 * 1024), 0, "Graph's unit");
        assert_eq!(DEFAULT_CHUNK_BYTES % (256 * 1024), 0, "Drive's unit");
    }

    // --- response headers reach finalize_upload ---------------------------

    /// S3's `PutObject` answers with an EMPTY body and the ETag in a header;
    /// an outcome of status + body alone left the adapter nothing to finalize
    /// with.
    #[test]
    fn response_headers_are_collected_for_the_adapter() {
        let mut raw = HeaderMap::new();
        raw.insert("ETag", HeaderValue::from_static("\"abc123\""));
        raw.insert("Location", HeaderValue::from_static("https://x.test/s/1"));
        let headers = collect_headers(&raw);
        assert_eq!(headers.get("etag").map(String::as_str), Some("\"abc123\""));
        assert_eq!(
            headers.get("location").map(String::as_str),
            Some("https://x.test/s/1")
        );
    }

    /// Lowercased so an adapter need not guess whether this provider wrote
    /// `ETag`, `Etag` or `etag` — HTTP says they are the same header.
    #[test]
    fn header_keys_are_lowercased() {
        let mut raw = HeaderMap::new();
        raw.insert("X-Goog-Upload-Status", HeaderValue::from_static("final"));
        let headers = collect_headers(&raw);
        assert_eq!(
            headers.keys().collect::<Vec<_>>(),
            vec!["x-goog-upload-status"]
        );
    }

    /// A repeated header is joined rather than dropped, so a provider that
    /// splits a value across two lines does not lose half of it.
    #[test]
    fn a_repeated_header_is_joined_not_lost() {
        let mut raw = HeaderMap::new();
        raw.append("x-amz-meta", HeaderValue::from_static("a"));
        raw.append("x-amz-meta", HeaderValue::from_static("b"));
        assert_eq!(
            collect_headers(&raw).get("x-amz-meta").map(String::as_str),
            Some("a, b")
        );
    }

    /// Bytes that are not UTF-8 cannot cross the QuickJS string boundary, so
    /// the header is skipped rather than mangled into a value the adapter
    /// would compare against and silently mismatch.
    #[test]
    fn a_non_utf8_header_is_skipped_rather_than_mangled() {
        let mut raw = HeaderMap::new();
        raw.insert("etag", HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap());
        raw.insert("content-type", HeaderValue::from_static("application/json"));
        let headers = collect_headers(&raw);
        assert!(!headers.contains_key("etag"));
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
    }
}
