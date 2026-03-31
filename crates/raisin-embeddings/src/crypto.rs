//! API key encryption using AES-256-GCM.
//!
//! This module provides secure encryption and decryption of API keys for
//! storing embedding provider credentials. It uses the ring crate for
//! cryptographic operations.

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
}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// API key encryptor using AES-256-GCM.
///
/// This struct provides methods to encrypt and decrypt API keys using a master key.
/// The encrypted data format is: [nonce (12 bytes)][ciphertext + tag]
///
/// # Security Notes
///
/// - Uses AES-256-GCM for authenticated encryption
/// - Generates a random nonce for each encryption operation
/// - The nonce is prepended to the ciphertext for storage
/// - Master key should be stored securely (environment variable, secrets manager, etc.)
pub struct ApiKeyEncryptor {
    master_key: [u8; 32],
}

impl ApiKeyEncryptor {
    /// Create a new encryptor with the given master key.
    ///
    /// # Arguments
    ///
    /// * `master_key` - A 32-byte master key for AES-256-GCM
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_embeddings::crypto::ApiKeyEncryptor;
    ///
    /// let master_key = [0u8; 32]; // In production, use a secure random key
    /// let encryptor = ApiKeyEncryptor::new(&master_key);
    /// ```
    pub fn new(master_key: &[u8; 32]) -> Self {
        Self {
            master_key: *master_key,
        }
    }

    /// Encrypt a plaintext API key.
    ///
    /// The result is [nonce (12 bytes)][ciphertext + authentication tag].
    ///
    /// # Arguments
    ///
    /// * `plaintext` - The API key to encrypt
    ///
    /// # Returns
    ///
    /// Encrypted bytes with nonce prepended
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_embeddings::crypto::ApiKeyEncryptor;
    ///
    /// let master_key = [0u8; 32];
    /// let encryptor = ApiKeyEncryptor::new(&master_key);
    /// let encrypted = encryptor.encrypt("sk-my-secret-api-key").unwrap();
    /// ```
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>> {
        let rng = SystemRandom::new();

        // Generate a random nonce
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|_| CryptoError::EncryptionFailed("Failed to generate nonce".to_string()))?;

        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        // Create encryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.master_key)
            .map_err(|_| CryptoError::EncryptionFailed("Failed to create key".to_string()))?;

        let key = LessSafeKey::new(unbound_key);

        // Prepare plaintext (we need to make a mutable copy with room for the tag)
        let mut in_out = plaintext.as_bytes().to_vec();

        // Seal the data (encrypt in place and append tag)
        key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CryptoError::EncryptionFailed("Sealing failed".to_string()))?;

        // Prepend nonce to the ciphertext
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);

        Ok(result)
    }

    /// Decrypt an encrypted API key.
    ///
    /// Expects data in the format: [nonce (12 bytes)][ciphertext + tag]
    ///
    /// # Arguments
    ///
    /// * `encrypted` - The encrypted data with nonce prepended
    ///
    /// # Returns
    ///
    /// The decrypted plaintext API key
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_embeddings::crypto::ApiKeyEncryptor;
    ///
    /// let master_key = [0u8; 32];
    /// let encryptor = ApiKeyEncryptor::new(&master_key);
    /// let encrypted = encryptor.encrypt("sk-my-secret-api-key").unwrap();
    /// let decrypted = encryptor.decrypt(&encrypted).unwrap();
    /// assert_eq!(decrypted, "sk-my-secret-api-key");
    /// ```
    pub fn decrypt(&self, encrypted: &[u8]) -> Result<String> {
        if encrypted.len() < 12 {
            return Err(CryptoError::DecryptionFailed(
                "Data too short (missing nonce)".to_string(),
            ));
        }

        // Extract nonce (first 12 bytes)
        let nonce_bytes: [u8; 12] = encrypted[..12]
            .try_into()
            .map_err(|_| CryptoError::DecryptionFailed("Invalid nonce".to_string()))?;

        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        // Create decryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.master_key)
            .map_err(|_| CryptoError::DecryptionFailed("Failed to create key".to_string()))?;

        let key = LessSafeKey::new(unbound_key);

        // Prepare mutable buffer for decryption (ciphertext + tag)
        let mut in_out = encrypted[12..].to_vec();

        // Decrypt in place
        let plaintext = key
            .open_in_place(nonce, Aad::empty(), &mut in_out)
            .map_err(|_| CryptoError::DecryptionFailed("Opening failed".to_string()))?;

        // Convert to string
        String::from_utf8(plaintext.to_vec()).map_err(CryptoError::Utf8Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let master_key = [42u8; 32];
        let encryptor = ApiKeyEncryptor::new(&master_key);

        let plaintext = "sk-test-api-key-1234567890";
        let encrypted = encryptor.encrypt(plaintext).unwrap();
        let decrypted = encryptor.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_encrypt_produces_different_ciphertexts() {
        let master_key = [42u8; 32];
        let encryptor = ApiKeyEncryptor::new(&master_key);

        let plaintext = "sk-test-api-key";
        let encrypted1 = encryptor.encrypt(plaintext).unwrap();
        let encrypted2 = encryptor.encrypt(plaintext).unwrap();

        // Different nonces should produce different ciphertexts
        assert_ne!(encrypted1, encrypted2);

        // But both should decrypt to the same plaintext
        assert_eq!(encryptor.decrypt(&encrypted1).unwrap(), plaintext);
        assert_eq!(encryptor.decrypt(&encrypted2).unwrap(), plaintext);
    }

    #[test]
    fn test_decrypt_with_wrong_key() {
        let master_key1 = [1u8; 32];
        let master_key2 = [2u8; 32];

        let encryptor1 = ApiKeyEncryptor::new(&master_key1);
        let encryptor2 = ApiKeyEncryptor::new(&master_key2);

        let plaintext = "sk-test-api-key";
        let encrypted = encryptor1.encrypt(plaintext).unwrap();

        // Decryption with wrong key should fail
        assert!(encryptor2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn test_decrypt_invalid_data() {
        let master_key = [42u8; 32];
        let encryptor = ApiKeyEncryptor::new(&master_key);

        // Too short
        assert!(encryptor.decrypt(&[0u8; 5]).is_err());

        // Random data
        let random_data = vec![0u8; 50];
        assert!(encryptor.decrypt(&random_data).is_err());
    }

    #[test]
    fn test_encrypt_empty_string() {
        let master_key = [42u8; 32];
        let encryptor = ApiKeyEncryptor::new(&master_key);

        let encrypted = encryptor.encrypt("").unwrap();
        let decrypted = encryptor.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_encrypt_unicode() {
        let master_key = [42u8; 32];
        let encryptor = ApiKeyEncryptor::new(&master_key);

        let plaintext = "sk-🔑-api-key-测试-🚀";
        let encrypted = encryptor.encrypt(plaintext).unwrap();
        let decrypted = encryptor.decrypt(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_encrypted_format() {
        let master_key = [42u8; 32];
        let encryptor = ApiKeyEncryptor::new(&master_key);

        let plaintext = "test";
        let encrypted = encryptor.encrypt(plaintext).unwrap();

        // Should have: 12 bytes (nonce) + plaintext length + 16 bytes (GCM tag)
        assert!(encrypted.len() >= 12 + 16);
    }
}
