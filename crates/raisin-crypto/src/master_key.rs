//! Master-key loading from environment.
//!
//! Two loaders with deliberately different semantics preserve the exact
//! behaviour of the two historical call sites they replace.

use raisin_error::Error;

/// Load the master key from `RAISIN_MASTER_KEY` only.
///
/// Hard-errors (via [`raisin_error::Error::Backend`]) if the variable is unset,
/// not valid hex, or not exactly 32 bytes. This mirrors the AI-provider
/// decryption path, which requires an explicit key with no fallback.
pub fn master_key() -> Result<[u8; 32], Error> {
    let hex = std::env::var("RAISIN_MASTER_KEY").map_err(|_| {
        Error::Backend("RAISIN_MASTER_KEY environment variable not set".to_string())
    })?;

    let bytes = hex::decode(&hex)
        .map_err(|e| Error::Backend(format!("Invalid RAISIN_MASTER_KEY: {}", e)))?;

    bytes
        .try_into()
        .map_err(|_| Error::Backend("RAISIN_MASTER_KEY must be 32 bytes".to_string()))
}

/// Load the master key from `RAISIN_MASTER_KEY`, falling back to the legacy
/// `EMBEDDING_MASTER_KEY`.
///
/// Returns:
/// - `Ok(Some(key))` when either variable is set and holds a valid 32-byte hex key,
/// - `Ok(None)` when neither variable is set (the caller decides whether a
///   dev-mode fallback is acceptable),
/// - `Err(`[`raisin_error::Error::Validation`]`)` when a value is present but not
///   valid hex or not 32 bytes.
///
/// This preserves the HTTP transport's behaviour, including the legacy
/// embedding fallback so existing embedding deployments keep decrypting.
pub fn master_key_with_embedding_fallback() -> Result<Option<[u8; 32]>, Error> {
    let hex_key = std::env::var("RAISIN_MASTER_KEY")
        .or_else(|_| std::env::var("EMBEDDING_MASTER_KEY"))
        .ok();

    match hex_key {
        Some(hex) => {
            let bytes = hex::decode(&hex).map_err(|e| {
                Error::Validation(format!("RAISIN_MASTER_KEY is not valid hex: {e}"))
            })?;
            let key: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
                Error::Validation(format!(
                    "RAISIN_MASTER_KEY must be 32 bytes (64 hex chars), got {} bytes",
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
    use std::sync::Mutex;

    // Env vars are process-global; serialise the tests that mutate them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear() {
        std::env::remove_var("RAISIN_MASTER_KEY");
        std::env::remove_var("EMBEDDING_MASTER_KEY");
    }

    const HEX_A: &str = "0000000000000000000000000000000000000000000000000000000000000000";
    const HEX_B: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn test_master_key_requires_var() {
        let _g = ENV_LOCK.lock().unwrap();
        clear();
        assert!(master_key().is_err());
        std::env::set_var("RAISIN_MASTER_KEY", HEX_A);
        assert_eq!(master_key().unwrap(), [0u8; 32]);
        clear();
    }

    #[test]
    fn test_master_key_rejects_bad_len() {
        let _g = ENV_LOCK.lock().unwrap();
        clear();
        std::env::set_var("RAISIN_MASTER_KEY", "aabb");
        assert!(master_key().is_err());
        clear();
    }

    #[test]
    fn test_fallback_precedence() {
        let _g = ENV_LOCK.lock().unwrap();
        clear();
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
        clear();
    }

    #[test]
    fn test_fallback_rejects_bad_hex() {
        let _g = ENV_LOCK.lock().unwrap();
        clear();
        std::env::set_var("RAISIN_MASTER_KEY", "zzzz");
        assert!(master_key_with_embedding_fallback().is_err());
        clear();
    }
}
