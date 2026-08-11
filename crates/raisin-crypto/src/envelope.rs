//! On-disk envelope formats and the sniffing rule between them.
//!
//! ```text
//! v1 (historical): [nonce 12][ct+tag]                    min 28 bytes
//! v2:              [magic 4]["RSB2"][key_id u16 BE][nonce 12][ct+tag]   min 34 bytes
//! ```
//!
//! # The sniff rule is length-aware, and why that matters
//!
//! v1 has no header, so its first four bytes are the first four bytes of a
//! random nonce. Those can be `RSB2` by chance (probability 2^-32). The rule
//! is therefore:
//!
//! 1. `len >= 34` **and** the first four bytes are `RSB2` → v2.
//! 2. otherwise `len >= 28` → v1.
//!
//! Clause 1's length test is load-bearing: a 28-byte v1 blob whose nonce starts
//! `RSB2` is unambiguously v1, because no v2 envelope can be that short.
//!
//! What the rule cannot fix: a **longer** v1 blob whose nonce starts `RSB2`
//! misparses as v2 and fails to open. That is a retroactive property of data
//! already at rest — v1 carries no discriminator, so no reader can do better.
//! At 2^-32 per blob it is ~1 in 4.3 billion. It is pinned by a test so nobody
//! "fixes" the sniff into something that breaks the common case.

use crate::error::{CryptoError, Result};

/// v2 envelope magic.
pub const MAGIC: [u8; 4] = *b"RSB2";

/// GCM tag length.
pub const TAG_LEN: usize = 16;
/// Nonce length.
pub const NONCE_LEN: usize = 12;
/// Shortest possible v1 blob: nonce + tag (empty plaintext).
pub const V1_MIN_LEN: usize = NONCE_LEN + TAG_LEN; // 28
/// Shortest possible v2 blob: magic + key id + nonce + tag (empty plaintext).
pub const V2_MIN_LEN: usize = MAGIC.len() + 2 + NONCE_LEN + TAG_LEN; // 34

/// A parsed envelope, borrowing the input buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Envelope<'a> {
    /// Historical format: no key id, no AAD.
    V1 {
        nonce: [u8; NONCE_LEN],
        body: &'a [u8],
    },
    /// Versioned format: key id and context-bound AAD.
    V2 {
        key_id: u16,
        nonce: [u8; NONCE_LEN],
        body: &'a [u8],
    },
}

impl Envelope<'_> {
    /// The key id an opener should use: `Some(id)` for v2, `None` for v1
    /// (which is un-attributed and must be opened by trial).
    pub fn key_id(&self) -> Option<u16> {
        match self {
            Envelope::V1 { .. } => None,
            Envelope::V2 { key_id, .. } => Some(*key_id),
        }
    }
}

/// Sniff and parse a blob as v2 or v1. See the module docs for the rule.
pub fn parse(bytes: &[u8]) -> Result<Envelope<'_>> {
    if bytes.len() >= V2_MIN_LEN && bytes[..4] == MAGIC {
        return parse_v2(bytes);
    }
    if bytes.len() < V1_MIN_LEN {
        return Err(CryptoError::decryption_failed(format!(
            "Blob too short: {} bytes, minimum {} (v1) / {} (v2)",
            bytes.len(),
            V1_MIN_LEN,
            V2_MIN_LEN
        )));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[..NONCE_LEN]);
    Ok(Envelope::V1 {
        nonce,
        body: &bytes[NONCE_LEN..],
    })
}

/// Parse strictly as v2, erroring with [`CryptoError::BadMagic`] otherwise.
pub fn parse_v2(bytes: &[u8]) -> Result<Envelope<'_>> {
    if bytes.len() < V2_MIN_LEN {
        return Err(CryptoError::decryption_failed(format!(
            "v2 blob too short: {} bytes, minimum {V2_MIN_LEN}",
            bytes.len()
        )));
    }
    if bytes[..4] != MAGIC {
        return Err(CryptoError::BadMagic {
            expected: MAGIC,
            got: bytes[..4].to_vec(),
        });
    }
    let key_id = u16::from_be_bytes([bytes[4], bytes[5]]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[6..6 + NONCE_LEN]);
    Ok(Envelope::V2 {
        key_id,
        nonce,
        body: &bytes[6 + NONCE_LEN..],
    })
}

/// Serialise a v1 envelope: `[nonce][ct+tag]`.
pub fn encode_v1(nonce: &[u8; NONCE_LEN], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(NONCE_LEN + body.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(body);
    out
}

/// Serialise a v2 envelope: `[RSB2][key_id BE][nonce][ct+tag]`.
pub fn encode_v2(key_id: u16, nonce: &[u8; NONCE_LEN], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(V2_MIN_LEN + body.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&key_id.to_be_bytes());
    out.extend_from_slice(nonce);
    out.extend_from_slice(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v2_roundtrip_layout() {
        let nonce = [7u8; NONCE_LEN];
        let blob = encode_v2(0x0002, &nonce, &[9u8; 20]);
        assert_eq!(&blob[..4], b"RSB2");
        assert_eq!(&blob[4..6], &[0x00, 0x02]);
        assert_eq!(&blob[6..18], &nonce);
        match parse(&blob).unwrap() {
            Envelope::V2 {
                key_id,
                nonce: n,
                body,
            } => {
                assert_eq!(key_id, 2);
                assert_eq!(n, nonce);
                assert_eq!(body, &[9u8; 20]);
            }
            other => panic!("expected v2, got {other:?}"),
        }
    }

    #[test]
    fn test_short_rsb2_blob_is_v1_by_length_rule() {
        // 28 bytes starting with RSB2: too short to be v2, so it is v1.
        let mut blob = b"RSB2".to_vec();
        blob.extend_from_slice(&[0u8; 24]);
        assert_eq!(blob.len(), V1_MIN_LEN);
        assert!(matches!(parse(&blob).unwrap(), Envelope::V1 { .. }));
    }

    #[test]
    fn test_too_short_is_error() {
        assert!(parse(&[0u8; 27]).is_err());
    }

    #[test]
    fn test_parse_v2_rejects_bad_magic() {
        let blob = [0u8; 40];
        assert!(matches!(parse_v2(&blob), Err(CryptoError::BadMagic { .. })));
    }
}
