// SPDX-License-Identifier: BSL-1.1

//! Serve one stored blob over HTTP, streamed, with ranges and validators.
//!
//! Nothing is buffered: the size comes from [`BinaryStorage::head`] and the
//! body from [`BinaryStorage::get_range`] / [`BinaryStorage::get_stream`], so
//! memory is one chunk whatever the blob's size. A response carries an exact
//! `Content-Length` and `Accept-Ranges: bytes` on every outcome, `206` with
//! `Content-Range` for a satisfiable range, `416` with `bytes */total` for an
//! unsatisfiable one. When the caller knows the blob's sha256 (a generated
//! artifact) the response also carries a strong `ETag` and `Repr-Digest`, and
//! `If-Range` is honoured: a range whose validator no longer matches gets the
//! whole, current representation — the only answer that cannot splice two
//! different files together on resume.

use super::http_range::{self, RangeResolution};
use crate::{error::ApiError, state::AppState};
use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use raisin_binary::BinaryStorage;

/// What to serve, and how to describe it.
pub(crate) struct BlobResponse<'a> {
    pub storage_key: &'a str,
    pub mime_type: &'a str,
    pub disposition: &'a str,
    pub cache_control: &'a str,
    /// Lower-case hex sha256 of the blob, when known.
    pub sha256: Option<&'a str>,
    pub range: Option<&'a str>,
    pub if_range: Option<&'a str>,
}

/// The strong entity tag for a blob with a known digest.
pub(crate) fn etag_for(sha256: &str) -> String {
    format!("\"sha256:{sha256}\"")
}

/// Does an `If-Range` value still name this representation? Only a strong
/// ETag can say so: a date is too coarse to vouch for byte-identical content
/// (RFC 9110 §13.1.5), so it — and anything when no ETag is known — means
/// "send the whole thing".
pub(crate) fn if_range_matches(if_range: Option<&str>, etag: Option<&str>) -> bool {
    match (if_range.map(str::trim), etag) {
        (None, _) => true,
        (Some(v), Some(tag)) => !v.starts_with("W/") && v == tag,
        (Some(_), None) => false,
    }
}

pub(crate) async fn serve_blob(
    state: &AppState,
    r: BlobResponse<'_>,
) -> Result<Response, ApiError> {
    let storage_err = |e: anyhow::Error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "STORAGE_ERROR",
            format!("Failed to retrieve file: {e}"),
        )
    };
    let meta = state
        .bin
        .head(r.storage_key)
        .await
        .map_err(storage_err)?
        .ok_or_else(|| ApiError::not_found("Stored file not found"))?;
    let total = meta.size;

    let etag = r.sha256.map(etag_for);
    let range = if if_range_matches(r.if_range, etag.as_deref()) {
        r.range
    } else {
        None
    };
    let resolution = http_range::resolve(range, total);

    let mut base = Response::builder()
        .header(header::CONTENT_TYPE, r.mime_type)
        .header(header::CONTENT_DISPOSITION, r.disposition)
        .header(header::CACHE_CONTROL, r.cache_control)
        // On EVERY outcome: a client decides whether it can seek or resume
        // from its first response.
        .header(header::ACCEPT_RANGES, "bytes");
    if let Some(tag) = &etag {
        base = base.header(header::ETAG, tag.as_str());
    }
    if let Some(digest) = r.sha256.and_then(raisin_binary::sha256_hex_to_base64) {
        // RFC 9530: the digest of the whole representation, also on a 206,
        // so a resumed download can be verified end to end.
        if let Ok(v) = HeaderValue::from_str(&format!("sha-256=:{digest}:")) {
            base = base.header("repr-digest", v);
        }
    }

    let response = match resolution {
        RangeResolution::Unsatisfiable => base
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
            .header(header::CONTENT_LENGTH, 0)
            .body(Body::empty()),
        RangeResolution::Satisfiable { start, end } => {
            let body = state
                .bin
                .get_range(r.storage_key, start..end + 1)
                .await
                .map_err(storage_err)?;
            base.status(StatusCode::PARTIAL_CONTENT)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{total}"),
                )
                // The length of the PART, not of the file.
                .header(header::CONTENT_LENGTH, end - start + 1)
                .body(Body::from_stream(body))
        }
        RangeResolution::None => {
            let body = state
                .bin
                .get_range(r.storage_key, 0..total)
                .await
                .map_err(storage_err)?;
            base.status(StatusCode::OK)
                .header(header::CONTENT_LENGTH, total)
                .body(Body::from_stream(body))
        }
    };
    Ok(response.expect("valid response with valid headers"))
}

/// `private, max-age=<seconds left on the signature>, no-transform`.
pub(crate) fn signed_cache_control(expires: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "private, max-age={}, no-transform",
        expires.saturating_sub(now)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn if_range_decision_table() {
        let tag = etag_for(&"a".repeat(64));
        // No If-Range: the Range stands.
        assert!(if_range_matches(None, Some(&tag)));
        assert!(if_range_matches(None, None));
        // The same strong tag: the Range stands.
        assert!(if_range_matches(Some(&tag), Some(&tag)));
        assert!(if_range_matches(Some(&format!(" {tag} ")), Some(&tag)));
        // Another blob (rebuilt): the whole thing.
        assert!(!if_range_matches(
            Some(&etag_for(&"b".repeat(64))),
            Some(&tag)
        ));
        // A weak tag or a date never vouches for bytes.
        assert!(!if_range_matches(Some(&format!("W/{tag}")), Some(&tag)));
        assert!(!if_range_matches(
            Some("Sat, 11 Oct 2026 10:00:00 GMT"),
            Some(&tag)
        ));
        // No validator known: an If-Range cannot be honoured.
        assert!(!if_range_matches(Some(&tag), None));
    }

    #[test]
    fn the_cache_lifetime_is_what_is_left_of_the_signature() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let cc = signed_cache_control(now + 3600);
        assert!(cc.starts_with("private, max-age=3"), "{cc}");
        assert!(cc.ends_with("no-transform"));
        assert_eq!(signed_cache_control(0), "private, max-age=0, no-transform");
    }
}
