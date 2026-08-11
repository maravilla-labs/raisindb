//! Master-key loading from environment.
//!
//! Both loaders now delegate to [`crate::keyring::EnvKeyProvider`], so a
//! deployment that has moved to `RAISIN_MASTER_KEYS` is understood by the
//! historical call sites too. Their observable semantics are unchanged:
//! `master_key` hard-errors when nothing is configured, and
//! `master_key_with_embedding_fallback` returns `Ok(None)` instead.

use raisin_error::Error;

use crate::keyring::{EnvKeyProvider, KeyProvider};

/// Load the active master key from the environment.
///
/// Hard-errors (via [`raisin_error::Error::Backend`]) when no key is
/// configured or the configuration is invalid. This mirrors the AI-provider
/// decryption path, which requires an explicit key with no fallback.
pub fn master_key() -> Result<[u8; 32], Error> {
    EnvKeyProvider::from_env()
        .map(|ring| ring.active().1)
        .map_err(|e| Error::Backend(e.to_string()))
}

/// Load the active master key, falling back to the legacy
/// `EMBEDDING_MASTER_KEY`.
///
/// Returns:
/// - `Ok(Some(key))` when a key is configured,
/// - `Ok(None)` when nothing is configured (the caller decides whether a
///   dev-mode fallback is acceptable),
/// - `Err(`[`raisin_error::Error::Validation`]`)` when a value is present but
///   invalid.
///
/// This preserves the HTTP transport's behaviour, including the legacy
/// embedding fallback so existing embedding deployments keep decrypting.
pub fn master_key_with_embedding_fallback() -> Result<Option<[u8; 32]>, Error> {
    if std::env::var_os("RAISIN_MASTER_KEY").is_some()
        || std::env::var_os("RAISIN_MASTER_KEYS").is_some()
    {
        let ring = EnvKeyProvider::from_env().map_err(|e| Error::Validation(e.to_string()))?;
        return Ok(Some(ring.active().1));
    }

    match std::env::var("EMBEDDING_MASTER_KEY").ok() {
        Some(hex_str) => {
            let bytes = hex::decode(hex_str.trim()).map_err(|e| {
                Error::Validation(format!("EMBEDDING_MASTER_KEY is not valid hex: {e}"))
            })?;
            let key: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
                Error::Validation(format!(
                    "EMBEDDING_MASTER_KEY must be 32 bytes (64 hex chars), got {} bytes",
                    v.len()
                ))
            })?;
            Ok(Some(key))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{clear_key_env, ENV_LOCK};

    const HEX_A: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const HEX_B: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn test_master_key_requires_var() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        assert!(master_key().is_err());
        std::env::set_var("RAISIN_MASTER_KEY", HEX_A);
        assert_eq!(master_key().unwrap(), [0u8; 32]);
        clear_key_env();
    }

    #[test]
    fn test_master_key_rejects_bad_len() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEY", "aabb");
        assert!(master_key().is_err());
        clear_key_env();
    }

    #[test]
    fn test_master_key_reads_the_ring() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", format!("1:{HEX_A},2:{HEX_B}"));
        std::env::set_var("RAISIN_MASTER_KEY_ACTIVE", "2");
        assert_eq!(master_key().unwrap(), [0x11u8; 32]);
        clear_key_env();
    }

    #[test]
    fn test_fallback_precedence() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        // Neither set -> Ok(None).
        assert_eq!(master_key_with_embedding_fallback().unwrap(), None);

        // Legacy only -> used.
        std::env::set_var("EMBEDDING_MASTER_KEY", HEX_B);
        assert_eq!(
            master_key_with_embedding_fallback().unwrap(),
            Some([0x11u8; 32])
        );

        // RAISIN_MASTER_KEY takes precedence over the legacy var.
        std::env::set_var("RAISIN_MASTER_KEY", HEX_A);
        assert_eq!(
            master_key_with_embedding_fallback().unwrap(),
            Some([0u8; 32])
        );
        clear_key_env();
    }

    #[test]
    fn test_fallback_rejects_bad_hex() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEY", "zzzz");
        assert!(master_key_with_embedding_fallback().is_err());
        clear_key_env();
    }
}
