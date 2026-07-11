//! Shared symmetric secret encryption for RaisinDB.
//!
//! This crate centralises the AES-256-GCM encryptor and master-key loading that
//! were previously duplicated across `raisin-ai` and `raisin-embeddings`. It is
//! the single source of truth for encrypting provider API keys and OAuth token
//! blobs at rest.
//!
//! # Wire format
//!
//! The on-disk / at-rest byte layout is `[nonce (12 bytes)][ciphertext + GCM
//! tag]`. This is intentionally **byte-identical** to the historical
//! `raisin-embeddings` / `raisin-ai` encryptor so that already-encrypted secrets
//! keep decrypting after the consolidation.
//!
//! # Master keys
//!
//! Two loaders are exposed with deliberately different semantics so that each
//! historical caller keeps its exact behaviour:
//!
//! - [`master_key`] — reads `RAISIN_MASTER_KEY` only and hard-errors if it is
//!   missing/invalid (the AI-provider decryption path).
//! - [`master_key_with_embedding_fallback`] — additionally accepts the legacy
//!   `EMBEDDING_MASTER_KEY`, and returns `Ok(None)` when neither is set so the
//!   caller can decide on a dev-mode fallback (the HTTP transport path).
//!
//! # Example
//!
//! ```
//! use raisin_crypto::SecretBox;
//!
//! let master_key = [0u8; 32]; // In production, use a securely generated key.
//! let sb = SecretBox::new(&master_key);
//! let encrypted = sb.encrypt("sk-my-secret-api-key").unwrap();
//! assert_eq!(sb.decrypt(&encrypted).unwrap(), "sk-my-secret-api-key");
//! ```

mod master_key;
mod secret_box;

pub use master_key::{master_key, master_key_with_embedding_fallback};
pub use secret_box::{ApiKeyEncryptor, CryptoError, Result, SecretBox};
