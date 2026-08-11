//! Shared symmetric secret encryption for RaisinDB.
//!
//! This crate is the single source of truth for encrypting secrets at rest:
//! provider API keys, OAuth token bundles, connector credentials, authserver
//! codes, and (soon) the node secret store.
//!
//! # Wire formats
//!
//! ```text
//! v1 (historical): [nonce 12][ciphertext + GCM tag]
//! v2:              ["RSB2"][key_id u16 BE][nonce 12][ciphertext + GCM tag]
//! ```
//!
//! v2 adds two things: a **key id**, so a ring of master keys can be rotated
//! without re-encrypting everything at once, and **AAD** derived from a
//! [`SecretContext`], so a blob is cryptographically bound to the tenant, repo,
//! subsystem and field it was written for.
//!
//! # Rolling upgrade: the reader ships first
//!
//! **[`SecretBox::seal`] emits v1 unless `RAISIN_CRYPTO_EMIT_V2` is set.** The
//! reader always accepts both. This is not a preference — a v2 blob is
//! unreadable by a peer still running the previous binary, so a cluster cannot
//! learn the format and start writing it in the same release without a window
//! in which some nodes cannot read what others wrote.
//!
//! Rollout: **R1 (this release)** everyone reads v2, nobody writes it.
//! **R2** the default flips to v2 emission and the env var goes away. Anyone
//! wanting v2 today can set the variable once every node is on R1.
//!
//! # What the context binding is worth
//!
//! For a family whose [`V1Policy`] is `Accept` — every pre-existing family, by
//! necessity — AAD is **accident detection, not attacker resistance**.
//! Stripping a v2 blob down to a v1 blob is deleting six bytes, and an
//! `Accept` reader will then open it with no context check at all. Only
//! [`V1Policy::Reject`] families ([`SecretContext::node_secret`],
//! [`SecretContext::authorization_code`]) get a binding that holds against a
//! chosen-blob attacker. Tightening a family to `Reject` requires its data at
//! rest to be fully migrated first.
//!
//! # Example
//!
//! ```
//! use raisin_crypto::{SecretBox, SecretContext};
//!
//! let master_key = [0u8; 32]; // In production, use a securely generated key.
//! let sb = SecretBox::new(&master_key);
//! let ctx = SecretContext::integration_tokens("acme", "site").unwrap();
//!
//! let sealed = sb.seal(&ctx, b"sk-my-secret-api-key").unwrap();
//! assert_eq!(sb.open(&ctx, &sealed).unwrap(), b"sk-my-secret-api-key");
//! ```
//!
//! # Master keys
//!
//! [`keyring::EnvKeyProvider::from_env`] reads `RAISIN_MASTER_KEYS` /
//! `RAISIN_MASTER_KEY_ACTIVE`, falling back to the legacy `RAISIN_MASTER_KEY`
//! (which loads at key id 0). Every configuration error surfaces there, at
//! startup — never at the first encrypt. Two thin wrappers,
//! [`master_key`] and [`master_key_with_embedding_fallback`], preserve the
//! historical call sites' error semantics.

pub mod context;
pub mod envelope;
mod error;
pub mod keyring;
mod master_key;
mod secret_box;

#[cfg(test)]
mod test_support;

pub use context::{SecretContext, V1Policy};
pub use envelope::{Envelope, MAGIC, V1_MIN_LEN, V2_MIN_LEN};
pub use error::{CryptoError, Result};
pub use keyring::{
    EnvKeyProvider, KeyMaterial, KeyProvider, Keyring, DEV_KEY_ID, EPHEMERAL_KEY_ID, LEGACY_KEY_ID,
};
pub use master_key::{master_key, master_key_with_embedding_fallback};
pub use secret_box::{ApiKeyEncryptor, SecretBox};
