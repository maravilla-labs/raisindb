use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use raisin_error::Result;
use regex::Regex;
use sha2::Sha256;
use std::sync::LazyLock;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

// Compile regexes once at startup for better performance and safety
static WHITESPACE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("Whitespace regex pattern is valid"));

static DASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-+").expect("Dash regex pattern is valid"));

/// Sanitize a node name to create a URL-safe slug
///
/// Converts input to lowercase, replaces whitespace with hyphens,
/// removes non-alphanumeric characters (except hyphens and underscores),
/// and trims leading/trailing hyphens.
///
/// # Arguments
/// * `input` - The raw name to sanitize
///
/// # Returns
/// * `Ok(String)` - The sanitized name
/// * `Err(...)` - If the input is empty, invalid, or results in an empty string
///
/// # Examples
/// ```
/// use raisin_core::sanitize_name;
///
/// assert_eq!(sanitize_name("Hello World").unwrap(), "hello-world");
/// assert_eq!(sanitize_name("Multi   spaces").unwrap(), "multi-spaces");
/// ```
pub fn sanitize_name(input: &str) -> Result<String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(raisin_error::Error::Validation("empty name".into()));
    }
    if s == ".." || s == "." {
        return Err(raisin_error::Error::Validation("invalid name".into()));
    }
    if s.contains('/') {
        return Err(raisin_error::Error::Validation("invalid name".into()));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(raisin_error::Error::Validation("invalid name".into()));
    }

    let lower = s.to_lowercase();
    let mut slug = WHITESPACE_RE.replace_all(&lower, "-").to_string();
    let filtered: String = slug
        .chars()
        .filter(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_' || *c == '.'
        })
        .collect();
    slug = DASH_RE.replace_all(&filtered, "-").to_string();
    slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        return Err(raisin_error::Error::Validation("empty name".into()));
    }
    Ok(slug)
}

// ============================================================================
// HMAC Signed URL Utilities
// ============================================================================

/// Generate an HMAC-SHA256 signature for an asset URL.
///
/// Creates a URL-safe base64-encoded signature binding the path, command, property path, and expiry.
///
/// # Arguments
/// * `secret` - The HMAC secret key (should be 32+ bytes)
/// * `path` - The full path being signed (e.g., "repo/branch/head/ws/path")
/// * `command` - The command type ("download" or "display")
/// * `property_path` - Optional property path (defaults to "file" if None)
/// * `expires` - Unix timestamp when the signature expires
///
/// # Returns
/// URL-safe base64-encoded signature string
///
/// # Example
/// ```
/// use raisin_core::sign_asset_url;
///
/// let secret = b"my-secret-key-32-bytes-or-more!!";
/// let sig = sign_asset_url(secret, "media/main/head/assets/file.jpg", "download", None, 1704067200);
/// assert!(!sig.is_empty());
///
/// // Sign with custom property path for thumbnails
/// let thumb_sig = sign_asset_url(secret, "media/main/head/assets/file.jpg", "display", Some("thumbnail"), 1704067200);
/// assert!(!thumb_sig.is_empty());
/// ```
pub fn sign_asset_url(
    secret: &[u8],
    path: &str,
    command: &str,
    property_path: Option<&str>,
    expires: u64,
) -> String {
    let prop = property_path.unwrap_or("file");
    let message = format!("{}:{}:{}:{}", path, command, prop, expires);
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(message.as_bytes());
    let result = mac.finalize();
    URL_SAFE_NO_PAD.encode(result.into_bytes())
}

/// Verify an HMAC-SHA256 signature for an asset URL.
///
/// Checks both expiry and signature validity using constant-time comparison.
///
/// # Arguments
/// * `secret` - The HMAC secret key (same as used for signing)
/// * `path` - The full path being verified
/// * `command` - The command type ("download" or "display")
/// * `property_path` - Optional property path (defaults to "file" if None)
/// * `expires` - Unix timestamp from the signature
/// * `signature` - The signature to verify (URL-safe base64)
///
/// # Returns
/// `true` if the signature is valid and not expired, `false` otherwise
///
/// # Example
/// ```
/// use raisin_core::{sign_asset_url, verify_asset_signature};
///
/// let secret = b"my-secret-key-32-bytes-or-more!!";
/// let path = "media/main/head/assets/file.jpg";
/// let expires = u64::MAX; // far future for test
/// let sig = sign_asset_url(secret, path, "download", None, expires);
///
/// assert!(verify_asset_signature(secret, path, "download", None, expires, &sig));
/// assert!(!verify_asset_signature(secret, path, "display", None, expires, &sig)); // wrong command
///
/// // Verify with custom property path
/// let thumb_sig = sign_asset_url(secret, path, "display", Some("thumbnail"), expires);
/// assert!(verify_asset_signature(secret, path, "display", Some("thumbnail"), expires, &thumb_sig));
/// ```
pub fn verify_asset_signature(
    secret: &[u8],
    path: &str,
    command: &str,
    property_path: Option<&str>,
    expires: u64,
    signature: &str,
) -> bool {
    // Check expiry first
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if now > expires {
        return false;
    }

    // Verify signature with constant-time comparison
    let expected = sign_asset_url(secret, path, command, property_path, expires);
    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_asset_url() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let sig = sign_asset_url(
            secret,
            "repo/main/head/ws/file.jpg",
            "download",
            None,
            1704067200,
        );
        assert!(!sig.is_empty());
        // Should be URL-safe base64
        assert!(!sig.contains('+'));
        assert!(!sig.contains('/'));
    }

    #[test]
    fn test_sign_asset_url_with_property_path() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let sig_default = sign_asset_url(
            secret,
            "repo/main/head/ws/file.jpg",
            "download",
            None,
            1704067200,
        );
        let sig_file = sign_asset_url(
            secret,
            "repo/main/head/ws/file.jpg",
            "download",
            Some("file"),
            1704067200,
        );
        let sig_thumb = sign_asset_url(
            secret,
            "repo/main/head/ws/file.jpg",
            "download",
            Some("thumbnail"),
            1704067200,
        );

        // None and "file" should produce the same signature (file is default)
        assert_eq!(sig_default, sig_file);
        // Different property path should produce different signature
        assert_ne!(sig_default, sig_thumb);
    }

    #[test]
    fn test_verify_valid_signature() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let path = "repo/main/head/ws/file.jpg";
        let expires = u64::MAX; // Far future
        let sig = sign_asset_url(secret, path, "download", None, expires);

        assert!(verify_asset_signature(
            secret, path, "download", None, expires, &sig
        ));
    }

    #[test]
    fn test_verify_with_property_path() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let path = "repo/main/head/ws/file.jpg";
        let expires = u64::MAX;
        let sig_thumb = sign_asset_url(secret, path, "display", Some("thumbnail"), expires);

        // Should verify with correct property path
        assert!(verify_asset_signature(
            secret,
            path,
            "display",
            Some("thumbnail"),
            expires,
            &sig_thumb
        ));
        // Should fail with wrong property path
        assert!(!verify_asset_signature(
            secret, path, "display", None, expires, &sig_thumb
        ));
        assert!(!verify_asset_signature(
            secret,
            path,
            "display",
            Some("file"),
            expires,
            &sig_thumb
        ));
    }

    #[test]
    fn test_verify_wrong_command() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let path = "repo/main/head/ws/file.jpg";
        let expires = u64::MAX;
        let sig = sign_asset_url(secret, path, "download", None, expires);

        // Wrong command should fail
        assert!(!verify_asset_signature(
            secret, path, "display", None, expires, &sig
        ));
    }

    #[test]
    fn test_verify_wrong_path() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let path = "repo/main/head/ws/file.jpg";
        let expires = u64::MAX;
        let sig = sign_asset_url(secret, path, "download", None, expires);

        // Wrong path should fail
        assert!(!verify_asset_signature(
            secret,
            "repo/main/head/ws/other.jpg",
            "download",
            None,
            expires,
            &sig
        ));
    }

    #[test]
    fn test_verify_expired() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let path = "repo/main/head/ws/file.jpg";
        let expires = 1; // Already expired (Unix epoch + 1 second)
        let sig = sign_asset_url(secret, path, "download", None, expires);

        // Expired should fail
        assert!(!verify_asset_signature(
            secret, path, "download", None, expires, &sig
        ));
    }

    #[test]
    fn test_verify_wrong_secret() {
        let secret = b"test-secret-key-32-bytes-long!!!";
        let wrong_secret = b"wrong-secret-key-32-bytes-long!!";
        let path = "repo/main/head/ws/file.jpg";
        let expires = u64::MAX;
        let sig = sign_asset_url(secret, path, "download", None, expires);

        // Wrong secret should fail
        assert!(!verify_asset_signature(
            wrong_secret,
            path,
            "download",
            None,
            expires,
            &sig
        ));
    }
}
