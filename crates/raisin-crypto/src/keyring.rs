//! Master-key rings: many keys, one active, resolvable by envelope key id.
//!
//! # Environment
//!
//! ```text
//! RAISIN_MASTER_KEYS="1:<64 hex>,2:<64 hex>"   # the ring
//! RAISIN_MASTER_KEY_ACTIVE="2"                 # which one seals
//! RAISIN_MASTER_KEY="<64 hex>"                 # legacy single key, id 0
//! ```
//!
//! Every configuration error is raised by [`EnvKeyProvider::from_env`], i.e. at
//! **startup**, not at the first encrypt. A ring that only explodes when
//! someone saves a credential is a ring that explodes in production.

use std::collections::BTreeMap;
use std::sync::Arc;

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::{CryptoError, Result};

/// Reserved id for the historical single `RAISIN_MASTER_KEY`.
pub const LEGACY_KEY_ID: u16 = 0x0000;
/// Reserved id for the insecure all-zero development key.
pub const DEV_KEY_ID: u16 = 0xFFFE;
/// Reserved id for a per-process ephemeral key (nothing sealed under it
/// survives a restart, by design).
pub const EPHEMERAL_KEY_ID: u16 = 0xFFFF;

/// 32 bytes of key material, wiped on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct KeyMaterial([u8; 32]);

impl KeyMaterial {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyMaterial(<redacted>)")
    }
}

/// Resolves envelope key ids to key material.
pub trait KeyProvider: Send + Sync {
    /// The key new seals use.
    fn active(&self) -> (u16, [u8; 32]);
    /// Look up a key by id.
    fn get(&self, id: u16) -> Option<[u8; 32]>;
    /// Every id this provider holds, ascending — used for error messages and
    /// for the v1 trial-open order.
    fn available_ids(&self) -> Vec<u16>;
    /// Whether this process is a development instance. Gates [`DEV_KEY_ID`].
    fn dev_mode(&self) -> bool {
        false
    }
}

/// An in-memory ring of master keys.
#[derive(Debug)]
pub struct Keyring {
    keys: BTreeMap<u16, KeyMaterial>,
    active: u16,
    dev_mode: bool,
}

impl Keyring {
    /// A ring holding exactly one key, at [`LEGACY_KEY_ID`].
    ///
    /// This is what `SecretBox::new(&key)` builds, which is why the ~25
    /// existing single-key call sites keep working unchanged.
    pub fn single(key: [u8; 32]) -> Self {
        Self::single_at(LEGACY_KEY_ID, key)
    }

    /// A ring holding exactly one key at an explicit id.
    pub fn single_at(id: u16, key: [u8; 32]) -> Self {
        let mut keys = BTreeMap::new();
        keys.insert(id, KeyMaterial::new(key));
        Self {
            keys,
            active: id,
            dev_mode: false,
        }
    }

    /// Build a ring from `(id, key)` pairs with an explicit active id.
    ///
    /// Errors on duplicate ids and on an active id that is not present.
    pub fn new(entries: Vec<(u16, [u8; 32])>, active: u16) -> Result<Self> {
        let mut keys: BTreeMap<u16, KeyMaterial> = BTreeMap::new();
        for (id, key) in entries {
            if keys.contains_key(&id) {
                return Err(CryptoError::KeyringConfig(format!(
                    "duplicate master key id {id}"
                )));
            }
            keys.insert(id, KeyMaterial::new(key));
        }
        if keys.is_empty() {
            return Err(CryptoError::KeyringConfig(
                "keyring is empty: no master keys configured".to_string(),
            ));
        }
        if !keys.contains_key(&active) {
            return Err(CryptoError::KeyringConfig(format!(
                "active key id {active} is not present in the keyring (available ids: {:?})",
                keys.keys().copied().collect::<Vec<_>>()
            )));
        }
        Ok(Self {
            keys,
            active,
            dev_mode: false,
        })
    }

    /// Mark this ring as belonging to a development instance, permitting
    /// [`DEV_KEY_ID`] blobs to be opened.
    pub fn with_dev_mode(mut self, dev_mode: bool) -> Self {
        self.dev_mode = dev_mode;
        self
    }

    /// A dev-only ring holding the all-zero key at [`DEV_KEY_ID`], in dev mode.
    ///
    /// Anything sealed under it is readable by anyone who knows the format.
    pub fn insecure_dev() -> Self {
        Self::single_at(DEV_KEY_ID, [0u8; 32]).with_dev_mode(true)
    }

    /// A ring with a freshly generated key at [`EPHEMERAL_KEY_ID`]. Nothing
    /// sealed under it can be opened after this process exits.
    pub fn ephemeral() -> Result<Self> {
        use ring::rand::{SecureRandom, SystemRandom};
        let mut key = [0u8; 32];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| CryptoError::KeyringConfig("failed to generate ephemeral key".into()))?;
        Ok(Self::single_at(EPHEMERAL_KEY_ID, key))
    }
}

impl KeyProvider for Keyring {
    fn active(&self) -> (u16, [u8; 32]) {
        let key = self
            .keys
            .get(&self.active)
            .expect("active key id is validated at construction");
        (self.active, key.expose())
    }

    fn get(&self, id: u16) -> Option<[u8; 32]> {
        self.keys.get(&id).map(|k| k.expose())
    }

    fn available_ids(&self) -> Vec<u16> {
        self.keys.keys().copied().collect()
    }

    fn dev_mode(&self) -> bool {
        self.dev_mode
    }
}

/// A [`Keyring`] loaded from the environment.
#[derive(Debug)]
pub struct EnvKeyProvider(Keyring);

impl EnvKeyProvider {
    /// Load `RAISIN_MASTER_KEYS` / `RAISIN_MASTER_KEY_ACTIVE`, falling back to
    /// the legacy `RAISIN_MASTER_KEY` (which loads at [`LEGACY_KEY_ID`]).
    ///
    /// Fails on: bad hex, wrong key length, malformed entries, duplicate ids,
    /// an active id that is absent, an empty configuration, and on id 0 being
    /// listed in `RAISIN_MASTER_KEYS` while `RAISIN_MASTER_KEY` is also set
    /// (two different sources for one id — silently picking either is worse
    /// than refusing).
    pub fn from_env() -> Result<Self> {
        let listed = std::env::var("RAISIN_MASTER_KEYS").ok();
        let legacy = std::env::var("RAISIN_MASTER_KEY").ok();

        let mut entries: Vec<(u16, [u8; 32])> = Vec::new();

        if let Some(list) = listed.as_deref() {
            for raw in list.split(',') {
                let item = raw.trim();
                if item.is_empty() {
                    continue;
                }
                let (id_s, key_s) = item.split_once(':').ok_or_else(|| {
                    CryptoError::KeyringConfig(format!(
                        "RAISIN_MASTER_KEYS entry '{item}' is not '<id>:<64 hex>'"
                    ))
                })?;
                let id = parse_key_id(id_s.trim()).ok_or_else(|| {
                    CryptoError::KeyringConfig(format!(
                        "RAISIN_MASTER_KEYS: '{id_s}' is not a valid key id (0..=65535)"
                    ))
                })?;
                let key = parse_hex_key(key_s.trim()).map_err(|e| {
                    CryptoError::KeyringConfig(format!("RAISIN_MASTER_KEYS id {id}: {e}"))
                })?;
                if id == LEGACY_KEY_ID && legacy.is_some() {
                    return Err(CryptoError::KeyringConfig(
                        "id 0 is listed in RAISIN_MASTER_KEYS while RAISIN_MASTER_KEY is also set; \
                         id 0 is reserved for the legacy key — remove one of them"
                            .to_string(),
                    ));
                }
                entries.push((id, key));
            }
        }

        if let Some(hex_key) = legacy.as_deref() {
            let key = parse_hex_key(hex_key.trim())
                .map_err(|e| CryptoError::KeyringConfig(format!("RAISIN_MASTER_KEY: {e}")))?;
            entries.push((LEGACY_KEY_ID, key));
        }

        if entries.is_empty() {
            return Err(CryptoError::KeyringConfig(
                "no master keys configured: set RAISIN_MASTER_KEYS or RAISIN_MASTER_KEY"
                    .to_string(),
            ));
        }

        let active = match std::env::var("RAISIN_MASTER_KEY_ACTIVE").ok() {
            Some(v) => parse_key_id(v.trim()).ok_or_else(|| {
                CryptoError::KeyringConfig(format!(
                    "RAISIN_MASTER_KEY_ACTIVE: '{v}' is not a valid key id (0..=65535)"
                ))
            })?,
            // No explicit choice: the legacy key if there is one, else the sole
            // listed key. Ambiguity is an error, never a guess.
            None => {
                if legacy.is_some() {
                    LEGACY_KEY_ID
                } else if entries.len() == 1 {
                    entries[0].0
                } else {
                    return Err(CryptoError::KeyringConfig(format!(
                        "RAISIN_MASTER_KEY_ACTIVE must be set when RAISIN_MASTER_KEYS holds more \
                         than one key (available ids: {:?})",
                        entries.iter().map(|(id, _)| *id).collect::<Vec<_>>()
                    )));
                }
            }
        };

        Ok(Self(Keyring::new(entries, active)?))
    }

    /// Mark this provider as a development instance.
    pub fn with_dev_mode(self, dev_mode: bool) -> Self {
        Self(self.0.with_dev_mode(dev_mode))
    }

    /// Load from the environment as a shared [`KeyProvider`].
    pub fn shared() -> Result<Arc<dyn KeyProvider>> {
        Ok(Arc::new(Self::from_env()?))
    }
}

impl KeyProvider for EnvKeyProvider {
    fn active(&self) -> (u16, [u8; 32]) {
        self.0.active()
    }
    fn get(&self, id: u16) -> Option<[u8; 32]> {
        self.0.get(id)
    }
    fn available_ids(&self) -> Vec<u16> {
        self.0.available_ids()
    }
    fn dev_mode(&self) -> bool {
        self.0.dev_mode()
    }
}

fn parse_key_id(s: &str) -> Option<u16> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u16>().ok()
    }
}

fn parse_hex_key(s: &str) -> std::result::Result<[u8; 32], String> {
    let bytes = hex::decode(s).map_err(|e| format!("invalid hex: {e}"))?;
    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("must be 32 bytes (64 hex chars), got {}", v.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{clear_key_env, ENV_LOCK};

    const HEX_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const HEX_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn test_env_two_key_ring() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", format!("1:{HEX_1},2:{HEX_2}"));
        std::env::set_var("RAISIN_MASTER_KEY_ACTIVE", "2");
        let ring = EnvKeyProvider::from_env().unwrap();
        assert_eq!(ring.available_ids(), vec![1, 2]);
        assert_eq!(ring.active().0, 2);
        assert_eq!(ring.get(1), Some([0x11u8; 32]));
        clear_key_env();
    }

    #[test]
    fn test_env_bad_hex_fails_at_startup() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", "1:zz");
        let err = EnvKeyProvider::from_env().unwrap_err().to_string();
        assert!(err.contains("invalid hex"), "{err}");
        clear_key_env();
    }

    #[test]
    fn test_env_duplicate_ids_fail() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", format!("1:{HEX_1},1:{HEX_2}"));
        std::env::set_var("RAISIN_MASTER_KEY_ACTIVE", "1");
        let err = EnvKeyProvider::from_env().unwrap_err().to_string();
        assert!(err.contains("duplicate master key id 1"), "{err}");
        clear_key_env();
    }

    #[test]
    fn test_env_active_names_absent_id() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", format!("1:{HEX_1}"));
        std::env::set_var("RAISIN_MASTER_KEY_ACTIVE", "7");
        let err = EnvKeyProvider::from_env().unwrap_err().to_string();
        assert!(err.contains("active key id 7"), "{err}");
        clear_key_env();
    }

    #[test]
    fn test_env_legacy_key_is_id_zero() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEY", HEX_1);
        let ring = EnvKeyProvider::from_env().unwrap();
        assert_eq!(ring.available_ids(), vec![0]);
        assert_eq!(ring.active(), (0, [0x11u8; 32]));
        clear_key_env();
    }

    #[test]
    fn test_env_id_zero_collision_with_legacy_fails() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", format!("0:{HEX_1}"));
        std::env::set_var("RAISIN_MASTER_KEY", HEX_2);
        let err = EnvKeyProvider::from_env().unwrap_err().to_string();
        assert!(err.contains("id 0 is listed"), "{err}");
        clear_key_env();
    }

    #[test]
    fn test_env_ambiguous_active_fails() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        std::env::set_var("RAISIN_MASTER_KEYS", format!("1:{HEX_1},2:{HEX_2}"));
        let err = EnvKeyProvider::from_env().unwrap_err().to_string();
        assert!(
            err.contains("RAISIN_MASTER_KEY_ACTIVE must be set"),
            "{err}"
        );
        clear_key_env();
    }

    #[test]
    fn test_env_nothing_configured_fails() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_key_env();
        assert!(EnvKeyProvider::from_env().is_err());
    }
}
