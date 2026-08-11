//! Shared test helpers. Env vars are process-global, so every test that
//! touches one must hold [`ENV_LOCK`] for its whole body.

use std::sync::Mutex;

pub static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Remove every environment variable this crate reads.
pub fn clear_key_env() {
    std::env::remove_var("RAISIN_MASTER_KEY");
    std::env::remove_var("RAISIN_MASTER_KEYS");
    std::env::remove_var("RAISIN_MASTER_KEY_ACTIVE");
    std::env::remove_var("EMBEDDING_MASTER_KEY");
    std::env::remove_var("RAISIN_CRYPTO_EMIT_V2");
}
