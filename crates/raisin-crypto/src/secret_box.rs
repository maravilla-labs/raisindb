//! AES-256-GCM authenticated encryption for secrets at rest.

use std::sync::Arc;

use base64::Engine as _;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};

use crate::context::{SecretContext, V1Policy};
use crate::envelope::{self, Envelope, NONCE_LEN};
use crate::error::{CryptoError, Result};
use crate::keyring::{KeyProvider, Keyring, DEV_KEY_ID, LEGACY_KEY_ID};

/// Symmetric secret encryptor using AES-256-GCM.
///
/// Reads both envelope versions; writes whichever the emit gate selects (see
/// [`SecretBox::seal`] and the crate docs).
///
/// # Security notes
///
/// - AES-256-GCM authenticated encryption; a fresh random nonce per operation.
/// - v2 binds the [`SecretContext`] as AAD, so a blob from one tenant/repo/field
///   does not open under another.
/// - Key material is held in a [`KeyProvider`] and wiped on drop.
pub struct SecretBox {
    keys: Arc<dyn KeyProvider>,
}

/// Backwards-compatible alias for [`SecretBox`].
///
/// Historical call sites referred to this type as `ApiKeyEncryptor`; the alias
/// keeps them compiling after the move into `raisin-crypto`.
pub type ApiKeyEncryptor = SecretBox;

/// Whether new seals emit envelope v2.
///
/// Read per call on purpose: a `OnceLock` would make the gate untestable and
/// would freeze an operator's mistake for the process lifetime. `getenv` is
/// negligible next to an AEAD seal.
fn emit_v2() -> bool {
    std::env::var_os("RAISIN_CRYPTO_EMIT_V2").is_some()
}

impl SecretBox {
    /// Create a secret box over a single master key.
    ///
    /// The key is installed as a one-key ring at [`LEGACY_KEY_ID`], which is
    /// what makes every historical single-key call site keep working (and keep
    /// producing key id 0 when v2 emission is on).
    pub fn new(master_key: &[u8; 32]) -> Self {
        Self {
            keys: Arc::new(Keyring::single(*master_key)),
        }
    }

    /// Create a secret box over a multi-key ring.
    pub fn with_keyring(keys: Arc<dyn KeyProvider>) -> Self {
        Self { keys }
    }

    /// The ring backing this box.
    pub fn keys(&self) -> &Arc<dyn KeyProvider> {
        &self.keys
    }

    // ---- v2 API ---------------------------------------------------------

    /// Seal `plaintext` under `ctx`.
    ///
    /// **Emits v1 unless `RAISIN_CRYPTO_EMIT_V2` is set.** A v2 blob is
    /// unreadable by a peer still running the previous binary, so a cluster
    /// cannot flip format in the same release that teaches it to read one. This
    /// release ships the reader; the next flips the default (see the crate
    /// docs).
    ///
    /// While the gate is off the context is accepted and ignored — there is no
    /// AAD in a v1 blob. Turning the gate on is what makes the binding real.
    pub fn seal(&self, ctx: &SecretContext, plaintext: &[u8]) -> Result<Vec<u8>> {
        let (key_id, key) = self.keys.active();
        let nonce = random_nonce()?;
        if emit_v2() {
            let body = seal_aead(&key, nonce, plaintext, &ctx.aad())?;
            Ok(envelope::encode_v2(key_id, &nonce, &body))
        } else {
            let body = seal_aead(&key, nonce, plaintext, &[])?;
            Ok(envelope::encode_v1(&nonce, &body))
        }
    }

    /// Open a blob of either envelope version.
    ///
    /// - **v2**: the key id selects the key and `ctx` supplies the AAD; a
    ///   mismatch in either is an authentication failure.
    /// - **v1**: `ctx` is *ignored* (v1 has no AAD) unless its policy is
    ///   [`V1Policy::Reject`], in which case this returns
    ///   [`CryptoError::V1Rejected`].
    ///
    /// # What AAD does and does not buy you here
    ///
    /// Wherever v1 is accepted, the context binding is **accident detection,
    /// not attacker resistance**. An attacker holding a v2 blob can strip it to
    /// v1 by deleting six bytes, and an `Accept` reader will open it with no
    /// context check at all. Only [`V1Policy::Reject`] — i.e. a family with no
    /// v1 data at rest — makes the binding load-bearing against a chosen-blob
    /// attacker.
    pub fn open(&self, ctx: &SecretContext, blob: &[u8]) -> Result<Vec<u8>> {
        match envelope::parse(blob)? {
            Envelope::V1 { nonce, body } => {
                if ctx.v1_policy() == V1Policy::Reject {
                    return Err(CryptoError::V1Rejected {
                        scope: ctx.scope().to_string(),
                    });
                }
                self.open_v1(nonce, body)
            }
            Envelope::V2 {
                key_id,
                nonce,
                body,
            } => {
                if key_id == DEV_KEY_ID && !self.keys.dev_mode() {
                    return Err(CryptoError::DevKeyOutsideDevMode { key_id });
                }
                let key = self
                    .keys
                    .get(key_id)
                    .ok_or_else(|| CryptoError::UnknownKeyId {
                        key_id,
                        available: self.keys.available_ids(),
                    })?;
                open_aead(&key, nonce, body, &ctx.aad()).map_err(|_| {
                    CryptoError::decryption_failed_for(
                        key_id,
                        "authentication failed (wrong key, wrong context, or corrupted bytes)",
                    )
                })
            }
        }
    }

    /// Seal a JSON value under `ctx`, returning base64 of the envelope.
    pub fn seal_json(&self, ctx: &SecretContext, value: &serde_json::Value) -> Result<String> {
        let bytes = serde_json::to_vec(value)?;
        let sealed = self.seal(ctx, &bytes)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(sealed))
    }

    /// Open a base64 blob produced by [`SecretBox::seal_json`].
    pub fn open_json(&self, ctx: &SecretContext, encoded: &str) -> Result<serde_json::Value> {
        let blob = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        let bytes = self.open(ctx, &blob)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Open an un-attributed v1 body by trying the ring's keys.
    ///
    /// v1 carries no key id, so the key must be found by trial. Order is:
    /// [`LEGACY_KEY_ID`] first (every v1 blob at rest predates the ring, so it
    /// was sealed with the single legacy key), then the active key, then the
    /// rest ascending. GCM's tag makes a wrong key a clean failure, never a
    /// wrong plaintext.
    fn open_v1(&self, nonce: [u8; NONCE_LEN], body: &[u8]) -> Result<Vec<u8>> {
        let (active_id, _) = self.keys.active();
        let mut order = vec![LEGACY_KEY_ID, active_id];
        order.extend(self.keys.available_ids());
        let mut tried = Vec::new();
        for id in order {
            if tried.contains(&id) {
                continue;
            }
            tried.push(id);
            let Some(key) = self.keys.get(id) else {
                continue;
            };
            if let Ok(plaintext) = open_aead(&key, nonce, body, &[]) {
                return Ok(plaintext);
            }
        }
        Err(CryptoError::decryption_failed(format!(
            "no key in the ring opens this v1 blob (tried ids: {:?})",
            self.keys.available_ids()
        )))
    }

    // ---- deprecated context-free API ------------------------------------

    /// Encrypt a plaintext string in the v1 format.
    #[deprecated(
        note = "context-free encryption: use SecretBox::seal with a SecretContext family constructor"
    )]
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>> {
        #[allow(deprecated)]
        self.encrypt_bytes(plaintext.as_bytes())
    }

    /// Encrypt raw bytes, returning `[nonce (12 bytes)][ciphertext + tag]`.
    #[deprecated(
        note = "context-free encryption: use SecretBox::seal with a SecretContext family constructor"
    )]
    pub fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let (_, key) = self.keys.active();
        let nonce = random_nonce()?;
        let body = seal_aead(&key, nonce, plaintext, &[])?;
        Ok(envelope::encode_v1(&nonce, &body))
    }

    /// Decrypt v1 data in the format `[nonce (12 bytes)][ciphertext + tag]`.
    #[deprecated(
        note = "context-free decryption: use SecretBox::open with a SecretContext family constructor"
    )]
    pub fn decrypt(&self, encrypted: &[u8]) -> Result<String> {
        #[allow(deprecated)]
        let plaintext = self.decrypt_bytes(encrypted)?;
        String::from_utf8(plaintext).map_err(CryptoError::Utf8Error)
    }

    /// Decrypt v1 data to raw bytes.
    ///
    /// A v2 blob fails here with an explicit message rather than a bare tag
    /// mismatch: a v2 blob's AAD comes from a context this API does not have,
    /// so it could never authenticate.
    #[deprecated(
        note = "context-free decryption: use SecretBox::open with a SecretContext family constructor"
    )]
    pub fn decrypt_bytes(&self, encrypted: &[u8]) -> Result<Vec<u8>> {
        match envelope::parse(encrypted)? {
            Envelope::V1 { nonce, body } => self.open_v1(nonce, body),
            Envelope::V2 { key_id, .. } => Err(CryptoError::decryption_failed_for(
                key_id,
                "this is a context-bound v2 envelope; open it with SecretBox::open(&ctx, ..)",
            )),
        }
    }

    /// Encrypt a JSON value, returning base64 of the v1 wire format.
    #[deprecated(note = "context-free encryption: use SecretBox::seal_json")]
    pub fn encrypt_json(&self, value: &serde_json::Value) -> Result<String> {
        let bytes = serde_json::to_vec(value)?;
        #[allow(deprecated)]
        let encrypted = self.encrypt_bytes(&bytes)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
    }

    /// Decrypt a base64-encoded blob produced by `encrypt_json`.
    #[deprecated(note = "context-free decryption: use SecretBox::open_json")]
    pub fn decrypt_json(&self, encoded: &str) -> Result<serde_json::Value> {
        let encrypted = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        #[allow(deprecated)]
        let bytes = self.decrypt_bytes(&encrypted)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

fn random_nonce() -> Result<[u8; NONCE_LEN]> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| CryptoError::EncryptionFailed("Failed to generate nonce".to_string()))?;
    Ok(nonce)
}

fn seal_aead(
    key: &[u8; 32],
    nonce: [u8; NONCE_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>> {
    let unbound = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| CryptoError::EncryptionFailed("Failed to create key".to_string()))?;
    let key = LessSafeKey::new(unbound);
    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(aad),
        &mut in_out,
    )
    .map_err(|_| CryptoError::EncryptionFailed("Sealing failed".to_string()))?;
    Ok(in_out)
}

fn open_aead(key: &[u8; 32], nonce: [u8; NONCE_LEN], body: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let unbound = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| CryptoError::decryption_failed("Failed to create key"))?;
    let key = LessSafeKey::new(unbound);
    let mut in_out = body.to_vec();
    let plaintext = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad),
            &mut in_out,
        )
        .map_err(|_| CryptoError::decryption_failed("Opening failed"))?;
    Ok(plaintext.to_vec())
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::test_support::{clear_key_env, ENV_LOCK};
    use crate::SecretContext;

    // ---- historical behaviour (pinned; must not change) -----------------

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let sb = SecretBox::new(&[42u8; 32]);
        let plaintext = "sk-test-api-key-1234567890";
        let encrypted = sb.encrypt(plaintext).unwrap();
        assert_eq!(sb.decrypt(&encrypted).unwrap(), plaintext);
    }

    #[test]
    fn test_encrypt_produces_different_ciphertexts() {
        let sb = SecretBox::new(&[42u8; 32]);
        let plaintext = "sk-test-api-key";
        let e1 = sb.encrypt(plaintext).unwrap();
        let e2 = sb.encrypt(plaintext).unwrap();
        assert_ne!(e1, e2);
        assert_eq!(sb.decrypt(&e1).unwrap(), plaintext);
        assert_eq!(sb.decrypt(&e2).unwrap(), plaintext);
    }

    #[test]
    fn test_decrypt_with_wrong_key() {
        let plaintext = "sk-test-api-key";
        let encrypted = SecretBox::new(&[1u8; 32]).encrypt(plaintext).unwrap();
        assert!(SecretBox::new(&[2u8; 32]).decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_decrypt_invalid_data() {
        let sb = SecretBox::new(&[42u8; 32]);
        assert!(sb.decrypt(&[0u8; 5]).is_err());
        assert!(sb.decrypt(&[0u8; 50]).is_err());
    }

    #[test]
    fn test_encrypt_empty_and_unicode() {
        let sb = SecretBox::new(&[42u8; 32]);
        let e = sb.encrypt("").unwrap();
        assert_eq!(sb.decrypt(&e).unwrap(), "");
        let u = "sk-🔑-api-key-测试-🚀";
        let e = sb.encrypt(u).unwrap();
        assert_eq!(sb.decrypt(&e).unwrap(), u);
    }

    #[test]
    fn test_wire_format_layout() {
        let sb = SecretBox::new(&[42u8; 32]);
        let encrypted = sb.encrypt("test").unwrap();
        // 12 bytes nonce + 4 bytes plaintext + 16 bytes GCM tag.
        assert_eq!(encrypted.len(), 12 + 4 + 16);
    }

    #[test]
    fn test_encrypt_json_roundtrip() {
        let sb = SecretBox::new(&[7u8; 32]);
        let value = serde_json::json!({
            "access_token": "abc123",
            "refresh_token": "def456",
            "expires_in": 3600,
            "scopes": ["read", "write"],
        });
        let encoded = sb.encrypt_json(&value).unwrap();
        assert_eq!(sb.decrypt_json(&encoded).unwrap(), value);
    }

    /// Fixed-vector test: a ciphertext produced by the historical
    /// `[nonce][ciphertext+tag]` layout (nonce = all-zero here for
    /// determinism) must still decrypt through the new `SecretBox`. This
    /// pins the wire format so old at-rest secrets keep working.
    #[test]
    fn test_fixed_vector_legacy_ciphertext_decrypts() {
        let sb = SecretBox::new(&[42u8; 32]);
        assert_eq!(
            sb.decrypt(&legacy_v1_fixture()).unwrap(),
            "sk-legacy-secret"
        );
    }

    /// Build the historical v1 blob for key `[42u8; 32]`, zero nonce.
    fn legacy_v1_fixture() -> Vec<u8> {
        let master_key = [42u8; 32];
        let nonce_bytes = [0u8; 12];
        let unbound = UnboundKey::new(&AES_256_GCM, &master_key).unwrap();
        let key = LessSafeKey::new(unbound);
        let mut in_out = b"sk-legacy-secret".to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::empty(),
            &mut in_out,
        )
        .unwrap();
        let mut legacy = nonce_bytes.to_vec();
        legacy.extend_from_slice(&in_out);
        legacy
    }

    // ---- v1 read path under the v2 reader -------------------------------

    #[test]
    fn test_v1_fixture_opens_under_v2_reader_with_context_ignored() {
        let sb = SecretBox::new(&[42u8; 32]);
        // Any Accept-policy context opens it: v1 has no AAD, so the context is
        // present and deliberately ignored.
        let a = SecretContext::integration_tokens("tenant-a", "repo-a").unwrap();
        let b = SecretContext::ai_provider("someone-else", "openai").unwrap();
        assert_eq!(
            sb.open(&a, &legacy_v1_fixture()).unwrap(),
            b"sk-legacy-secret"
        );
        assert_eq!(
            sb.open(&b, &legacy_v1_fixture()).unwrap(),
            b"sk-legacy-secret"
        );
    }

    #[test]
    fn test_v1_policy_reject_vs_accept() {
        let sb = SecretBox::new(&[42u8; 32]);
        let accept = SecretContext::integration_tokens("t", "r").unwrap();
        let reject = accept.clone().with_v1_policy(V1Policy::Reject);
        assert!(sb.open(&accept, &legacy_v1_fixture()).is_ok());
        assert!(matches!(
            sb.open(&reject, &legacy_v1_fixture()),
            Err(CryptoError::V1Rejected { .. })
        ));
    }

    // ---- v2 round trip and AAD binding ----------------------------------

    /// Seal as v2 regardless of the emit gate, for tests that exercise the
    /// v2 path itself. Does not touch the environment.
    fn seal_v2(sb: &SecretBox, ctx: &SecretContext, pt: &[u8]) -> Vec<u8> {
        let (key_id, key) = sb.keys.active();
        let nonce = random_nonce().unwrap();
        let body = seal_aead(&key, nonce, pt, &ctx.aad()).unwrap();
        envelope::encode_v2(key_id, &nonce, &body)
    }

    #[test]
    fn test_v2_roundtrip_identical_context() {
        let sb = SecretBox::new(&[3u8; 32]);
        let ctx = SecretContext::integration_tokens("acme", "site").unwrap();
        let blob = seal_v2(&sb, &ctx, b"hunter2");
        assert_eq!(sb.open(&ctx, &blob).unwrap(), b"hunter2");
    }

    // Four separate tests, one per AAD component. A single combined test
    // would still pass if the impl dropped one component from the AAD.

    #[test]
    fn test_v2_tenant_mismatch_fails() {
        let sb = SecretBox::new(&[3u8; 32]);
        let w = SecretContext::integration_tokens("tenant-a", "repo").unwrap();
        let r = SecretContext::integration_tokens("tenant-b", "repo").unwrap();
        let blob = seal_v2(&sb, &w, b"secret");
        assert!(sb.open(&r, &blob).is_err());
    }

    #[test]
    fn test_v2_repo_mismatch_fails() {
        let sb = SecretBox::new(&[3u8; 32]);
        let w = SecretContext::integration_tokens("tenant", "repo-a").unwrap();
        let r = SecretContext::integration_tokens("tenant", "repo-b").unwrap();
        let blob = seal_v2(&sb, &w, b"secret");
        assert!(sb.open(&r, &blob).is_err());
    }

    #[test]
    fn test_v2_scope_mismatch_fails() {
        let sb = SecretBox::new(&[3u8; 32]);
        // Same tenant/repo/field shape, different scope component.
        let w = SecretContext::mcp_tokens("tenant", "repo").unwrap();
        let r = SecretContext::integration_tokens("tenant", "repo").unwrap();
        let blob = seal_v2(&sb, &w, b"secret");
        assert!(sb.open(&r, &blob).is_err());
    }

    #[test]
    fn test_v2_field_mismatch_fails() {
        let sb = SecretBox::new(&[3u8; 32]);
        // Same tenant/repo/scope, different field component.
        let w = SecretContext::integration_tokens("tenant", "repo").unwrap();
        let r = SecretContext::integration_client_secret("tenant", "repo").unwrap();
        let blob = seal_v2(&sb, &w, b"secret");
        assert!(sb.open(&r, &blob).is_err());
    }

    #[test]
    fn test_cross_family_blob_does_not_open() {
        let sb = SecretBox::new(&[3u8; 32]);
        let w = SecretContext::integration_tokens("t", "r").unwrap();
        let r = SecretContext::authorization_code().unwrap();
        let blob = seal_v2(&sb, &w, b"code");
        assert!(sb.open(&r, &blob).is_err());
    }

    // ---- keyring --------------------------------------------------------

    #[test]
    fn test_two_key_ring_seals_with_active_and_opens_by_id() {
        let full = Arc::new(Keyring::new(vec![(1, [1u8; 32]), (2, [2u8; 32])], 2).unwrap())
            as Arc<dyn KeyProvider>;
        let sb = SecretBox::with_keyring(full.clone());
        let ctx = SecretContext::node_secret("t", "r").unwrap();
        let blob = seal_v2(&sb, &ctx, b"payload");
        assert_eq!(&blob[4..6], &[0x00, 0x02], "sealed under the active key");
        assert_eq!(sb.open(&ctx, &blob).unwrap(), b"payload");

        // A peer that only holds key 1 names the missing id and lists its own.
        let partial = Arc::new(Keyring::new(vec![(1, [1u8; 32])], 1).unwrap());
        let peer = SecretBox::with_keyring(partial);
        let err = peer.open(&ctx, &blob).unwrap_err();
        assert!(
            matches!(err, CryptoError::UnknownKeyId { key_id: 2, .. }),
            "{err}"
        );
        let msg = err.to_string();
        assert!(msg.contains('2'), "{msg}");
        assert!(msg.contains("[1]"), "{msg}");
    }

    #[test]
    fn test_dev_key_refused_outside_dev_mode() {
        let ctx = SecretContext::node_secret("t", "r").unwrap();
        let dev = SecretBox::with_keyring(Arc::new(Keyring::insecure_dev()));
        let blob = seal_v2(&dev, &ctx, b"dev-payload");
        assert_eq!(dev.open(&ctx, &blob).unwrap(), b"dev-payload");

        let prod = SecretBox::with_keyring(Arc::new(
            Keyring::single_at(DEV_KEY_ID, [0u8; 32]), // holds the key, not dev mode
        ));
        let err = prod.open(&ctx, &blob).unwrap_err();
        assert!(
            matches!(err, CryptoError::DevKeyOutsideDevMode { .. }),
            "{err}"
        );
        assert!(err.to_string().contains("DEV KEY"), "{err}");
    }

    // ---- envelope pins --------------------------------------------------

    #[test]
    fn test_v2_envelope_layout_pin() {
        let ring = Keyring::new(vec![(1, [1u8; 32]), (2, [2u8; 32])], 2).unwrap();
        let sb = SecretBox::with_keyring(Arc::new(ring));
        let ctx = SecretContext::node_secret("t", "r").unwrap();
        let pt = b"0123456789";
        let blob = seal_v2(&sb, &ctx, pt);
        assert_eq!(&blob[0..4], b"RSB2");
        assert_eq!(&blob[4..6], &[0x00, 0x02]);
        assert_eq!(blob.len(), 4 + 2 + 12 + pt.len() + 16);
    }

    /// A v1 blob whose random nonce happens to begin "RSB2" misparses as v2.
    ///
    /// This is UNFIXABLE retroactively: v1 has no discriminator, so its first
    /// four bytes are nonce entropy and can collide with any magic we choose.
    /// Probability 2^-32 per blob. Documented and pinned so nobody "fixes" the
    /// sniff rule in a way that breaks the common case instead.
    #[test]
    fn test_v1_nonce_starting_with_magic_misparses_as_v2() {
        let key = [5u8; 32];
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(b"RSB2");
        let body = seal_aead(&key, nonce, b"unlucky-secret", &[]).unwrap();
        let blob = envelope::encode_v1(&nonce, &body);
        assert!(blob.len() > envelope::V2_MIN_LEN);

        let sb = SecretBox::new(&key);
        let ctx = SecretContext::integration_tokens("t", "r").unwrap();
        // Read as v2: key id comes from nonce bytes 4..6, which are zero here,
        // so it resolves to id 0 and then fails authentication.
        let err = sb.open(&ctx, &blob).unwrap_err();
        assert!(
            matches!(
                err,
                CryptoError::DecryptionFailed {
                    key_id: Some(0),
                    ..
                }
            ),
            "{err}"
        );
    }

    /// The other direction: 28 bytes beginning "RSB2" is too short to be a v2
    /// envelope, so the length-aware rule reads it as v1 — correctly.
    #[test]
    fn test_short_blob_beginning_with_magic_is_read_as_v1() {
        let key = [6u8; 32];
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(b"RSB2");
        let body = seal_aead(&key, nonce, b"", &[]).unwrap(); // 16-byte tag only
        let blob = envelope::encode_v1(&nonce, &body);
        assert_eq!(blob.len(), 28);

        let sb = SecretBox::new(&key);
        let ctx = SecretContext::integration_tokens("t", "r").unwrap();
        assert_eq!(sb.open(&ctx, &blob).unwrap(), b"");
    }

    // ---- emit gate ------------------------------------------------------

    #[test]
    fn test_seal_emits_v1_by_default() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        let sb = SecretBox::new(&[9u8; 32]);
        let ctx = SecretContext::integration_tokens("t", "r").unwrap();
        let blob = sb.seal(&ctx, b"rolling-upgrade").unwrap();
        assert!(
            matches!(envelope::parse(&blob).unwrap(), Envelope::V1 { .. }),
            "seal() must emit v1 while RAISIN_CRYPTO_EMIT_V2 is unset"
        );
        assert_eq!(blob.len(), 12 + b"rolling-upgrade".len() + 16);
        assert_eq!(sb.open(&ctx, &blob).unwrap(), b"rolling-upgrade");
    }

    #[test]
    fn test_seal_emits_v2_when_gate_set() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1");
        let sb = SecretBox::new(&[9u8; 32]);
        let ctx = SecretContext::integration_tokens("t", "r").unwrap();
        let blob = sb.seal(&ctx, b"bound").unwrap();
        // SecretBox::new is a one-key ring at the legacy id, so key id is 0.
        assert!(matches!(
            envelope::parse(&blob).unwrap(),
            Envelope::V2 { key_id: 0, .. }
        ));
        assert_eq!(sb.open(&ctx, &blob).unwrap(), b"bound");
        let other = SecretContext::integration_tokens("t", "other").unwrap();
        assert!(sb.open(&other, &blob).is_err());
        clear_key_env();
    }

    #[test]
    fn test_legacy_env_key_seals_with_key_id_zero() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var(
            "RAISIN_MASTER_KEY",
            "1111111111111111111111111111111111111111111111111111111111111111",
        );
        std::env::set_var("RAISIN_CRYPTO_EMIT_V2", "1");
        let ring = crate::keyring::EnvKeyProvider::from_env().unwrap();
        assert_eq!(ring.active().0, LEGACY_KEY_ID);
        let sb = SecretBox::with_keyring(Arc::new(ring));
        let ctx = SecretContext::node_secret("t", "r").unwrap();
        let blob = sb.seal(&ctx, b"legacy").unwrap();
        assert_eq!(&blob[4..6], &[0x00, 0x00]);
        assert_eq!(sb.open(&ctx, &blob).unwrap(), b"legacy");
        clear_key_env();
    }
}
