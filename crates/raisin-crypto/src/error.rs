//! Error type for the crypto crate.
//!
//! # Why there is no `ContextMismatch`
//!
//! AES-GCM authenticates the ciphertext, the nonce **and** the AAD under one
//! tag. A tag mismatch is a single, undifferentiated failure: the crate cannot
//! tell "right key, wrong context" from "wrong key" from "corrupted bytes".
//! A `ContextMismatch` variant would therefore be a lie in two of those three
//! cases, and would invite callers to branch on it. Instead there is one
//! [`CryptoError::DecryptionFailed`], carrying the key id that was parsed out
//! of the envelope (when there was one) purely for diagnostics.

use thiserror::Error;

/// Errors that can occur during encryption/decryption operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    /// Authentication failed. Could be the wrong key, the wrong
    /// [`crate::SecretContext`], or corrupted bytes — AES-GCM cannot tell
    /// these apart. `key_id` is whatever the envelope claimed, for logs only.
    #[error("Decryption failed (envelope key id: {key_id:?}): {msg}")]
    DecryptionFailed { key_id: Option<u16>, msg: String },

    #[error("Invalid key length (expected 32 bytes)")]
    InvalidKeyLength,

    /// The envelope named a key id this keyring does not hold.
    #[error(
        "Unknown key id {key_id}: this keyring does not hold it (available ids: {available:?})"
    )]
    UnknownKeyId { key_id: u16, available: Vec<u16> },

    /// A blob that was required to be a v2 envelope did not start with `RSB2`.
    #[error("Bad envelope magic: expected {expected:?}, got {got:?}")]
    BadMagic { expected: [u8; 4], got: Vec<u8> },

    /// A v1 (context-free) blob was presented where the context demands v2.
    #[error("Refusing to open a v1 (unauthenticated-context) blob for scope '{scope}': this context is v1-Reject")]
    V1Rejected { scope: String },

    /// A blob sealed with the insecure all-zero development key was presented
    /// to a keyring that is not in dev mode.
    #[error("Refusing to open a blob sealed with the INSECURE DEV KEY (id {key_id:#06x}) because this keyring is not in dev mode")]
    DevKeyOutsideDevMode { key_id: u16 },

    /// Keyring configuration (usually environment) is invalid.
    #[error("Keyring configuration error: {0}")]
    KeyringConfig(String),

    #[error("UTF-8 error: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A [`crate::SecretContext`] component contained a NUL byte.
    #[error("Invalid secret context: component '{component}' contains a NUL byte")]
    InvalidContext { component: &'static str },
}

impl CryptoError {
    /// Decryption failure with no key id (v1 envelope, or pre-parse).
    pub(crate) fn decryption_failed(msg: impl Into<String>) -> Self {
        Self::DecryptionFailed {
            key_id: None,
            msg: msg.into(),
        }
    }

    /// Decryption failure attributable to a specific envelope key id.
    pub(crate) fn decryption_failed_for(key_id: u16, msg: impl Into<String>) -> Self {
        Self::DecryptionFailed {
            key_id: Some(key_id),
            msg: msg.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, CryptoError>;
