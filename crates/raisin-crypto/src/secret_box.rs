//! AES-256-GCM authenticated encryption for secrets at rest.

use base64::Engine as _;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use thiserror::Error;

/// Errors that can occur during encryption/decryption operations.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Invalid key length (expected 32 bytes)")]
    InvalidKeyLength,

    #[error("UTF-8 error: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[error("Base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// Symmetric secret encryptor using AES-256-GCM.
///
/// The encrypted byte layout is `[nonce (12 bytes)][ciphertext + tag]`.
///
/// # Security notes
///
/// - Uses AES-256-GCM for authenticated encryption.
/// - Generates a fresh random nonce for every encryption operation.
/// - The nonce is prepended to the ciphertext for storage.
/// - The master key should be stored securely (environment variable, secrets
///   manager, etc.).
pub struct SecretBox {
    master_key: [u8; 32],
}

/// Backwards-compatible alias for [`SecretBox`].
///
/// Historical call sites referred to this type as `ApiKeyEncryptor`; the alias
/// keeps them compiling after the move into `raisin-crypto`.
pub type ApiKeyEncryptor = SecretBox;

impl SecretBox {
    /// Create a new secret box with the given 32-byte master key.
    pub fn new(master_key: &[u8; 32]) -> Self {
        Self {
            master_key: *master_key,
        }
    }

    /// Encrypt a plaintext string.
    ///
    /// The result is `[nonce (12 bytes)][ciphertext + authentication tag]`.
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>> {
        self.encrypt_bytes(plaintext.as_bytes())
    }

    /// Encrypt raw bytes, returning `[nonce (12 bytes)][ciphertext + tag]`.
    pub fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let rng = SystemRandom::new();

        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| CryptoError::EncryptionFailed("Failed to generate nonce".to_string()))?;

        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.master_key)
            .map_err(|_| CryptoError::EncryptionFailed("Failed to create key".to_string()))?;
        let key = LessSafeKey::new(unbound_key);

        let mut in_out = plaintext.to_vec();
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CryptoError::EncryptionFailed("Sealing failed".to_string()))?;

        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        Ok(result)
    }

    /// Decrypt data in the format `[nonce (12 bytes)][ciphertext + tag]`.
    pub fn decrypt(&self, encrypted: &[u8]) -> Result<String> {
        let plaintext = self.decrypt_bytes(encrypted)?;
        String::from_utf8(plaintext).map_err(CryptoError::Utf8Error)
    }

    /// Decrypt data to raw bytes.
    pub fn decrypt_bytes(&self, encrypted: &[u8]) -> Result<Vec<u8>> {
        if encrypted.len() < 12 {
            return Err(CryptoError::DecryptionFailed(
                "Data too short (missing nonce)".to_string(),
            ));
        }

        let nonce_bytes: [u8; 12] = encrypted[..12]
            .try_into()
            .map_err(|_| CryptoError::DecryptionFailed("Invalid nonce".to_string()))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.master_key)
            .map_err(|_| CryptoError::DecryptionFailed("Failed to create key".to_string()))?;
        let key = LessSafeKey::new(unbound_key);

        let mut in_out = encrypted[12..].to_vec();
        let plaintext = key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CryptoError::DecryptionFailed("Opening failed".to_string()))?;

        Ok(plaintext.to_vec())
    }

    /// Encrypt a JSON value, returning base64 of the standard wire format.
    ///
    /// Intended for token blobs (e.g. OAuth access/refresh token bundles).
    pub fn encrypt_json(&self, value: &serde_json::Value) -> Result<String> {
        let bytes = serde_json::to_vec(value)?;
        let encrypted = self.encrypt_bytes(&bytes)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
    }

    /// Decrypt a base64-encoded blob produced by [`SecretBox::encrypt_json`].
    pub fn decrypt_json(&self, encoded: &str) -> Result<serde_json::Value> {
        let encrypted = base64::engine::general_purpose::STANDARD.decode(encoded)?;
        let bytes = self.decrypt_bytes(&encrypted)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Reproduce exactly what the old code produced: seal with an explicit
        // nonce, then prepend the nonce — identical to the historical impl.
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

        // New decryptor recovers the original plaintext.
        let sb = SecretBox::new(&master_key);
        assert_eq!(sb.decrypt(&legacy).unwrap(), "sk-legacy-secret");
    }
}
