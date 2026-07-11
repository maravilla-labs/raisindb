//! API key encryption using AES-256-GCM.
//!
//! The implementation now lives in the shared [`raisin_crypto`] crate; this
//! module re-exports it so existing `raisin_embeddings::crypto::ApiKeyEncryptor`
//! call sites keep compiling. The wire format is unchanged.

pub use raisin_crypto::{ApiKeyEncryptor, CryptoError, Result, SecretBox};
