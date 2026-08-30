//! The one declaration of the secret material RaisinDB reads from the
//! environment.
//!
//! Before this module the rules lived in four hand-rolled copies — the
//! production preflight in `raisin-server`'s `main.rs`, the auth-service
//! bootstrap in `startup/storage.rs`, the WebSocket transport's own
//! `JWT_SECRET` read, and the HTTP transport's `get_signing_secret` — each
//! re-deriving "which variable, which insecure default, required or not" on
//! its own. They had already drifted in a way that could not start a
//! production node: the preflight demanded `RAISIN_MASTER_KEY` or
//! `EMBEDDING_MASTER_KEY` by *name*, while the loader every consumer actually
//! calls ([`crate::master_key_with_embedding_fallback`]) has accepted the
//! `RAISIN_MASTER_KEYS` keyring since it was unified. A deployment that had
//! migrated to the keyring therefore exited 1 at boot demanding a variable
//! nothing would have read.
//!
//! Two rules keep that from coming back:
//!
//! 1. **The preflight validates through the real loaders, never by name.**
//!    [`production_secret_problems`] calls the same functions the running
//!    server calls, so a variable the loader learns to accept is accepted at
//!    boot on the same day, and a value that is present but malformed fails
//!    at boot rather than on the first request that needs it.
//! 2. **Every problem is reported in one pass.** The old preflight was a
//!    sequence of `if missing { error; exit(1) }`, so an operator with three
//!    unset variables discovered them one boot at a time. Callers get the
//!    whole list and exit once.

use crate::master_key_with_embedding_fallback;

/// The historical placeholder `JWT_SECRET`. Accepted only in `--dev-mode`;
/// treated as "not configured" everywhere else.
pub const INSECURE_JWT_DEFAULT: &str = "default_jwt_secret_change_in_production";

/// The `--dev-mode` fallback for `RAISINDB_SIGNING_SECRET`.
pub const DEV_SIGNING_SECRET: &[u8] = b"raisindb-dev-signing-secret-key!";

/// The JWT signing secret, or `None` when nothing usable is configured.
///
/// Unset, empty and the historical placeholder all collapse to `None` — a
/// caller in dev-mode substitutes [`INSECURE_JWT_DEFAULT`], and a caller in
/// production refuses to start. Doing that collapse here is what stops one
/// call site from treating the placeholder as a real secret.
pub fn jwt_secret() -> Option<String> {
    match std::env::var("JWT_SECRET") {
        Ok(s) if !s.is_empty() && s != INSECURE_JWT_DEFAULT => Some(s),
        _ => None,
    }
}

/// The HMAC key for signed asset URLs, or `None` when nothing is configured.
///
/// An empty value counts as unconfigured: an empty HMAC key signs every URL
/// identically, which is the failure the variable exists to prevent.
pub fn signing_secret() -> Option<Vec<u8>> {
    match std::env::var("RAISINDB_SIGNING_SECRET") {
        Ok(s) if !s.is_empty() => Some(s.into_bytes()),
        _ => None,
    }
}

/// One unmet production requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretProblem {
    /// The variable an operator should set (the canonical one when several
    /// spellings are accepted).
    pub var: &'static str,
    /// What is wrong: absent, or present and unusable.
    pub detail: String,
    /// What to put in it.
    pub remedy: &'static str,
}

impl std::fmt::Display for SecretProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} — {}", self.var, self.detail, self.remedy)
    }
}

/// Every production secret requirement that is not met, in one pass.
///
/// An empty vector means the process may boot outside `--dev-mode`. The
/// checks run through the same loaders the server uses at runtime, so this
/// cannot drift from what the running process will accept.
pub fn production_secret_problems() -> Vec<SecretProblem> {
    let mut problems = Vec::new();

    if jwt_secret().is_none() {
        problems.push(SecretProblem {
            var: "JWT_SECRET",
            detail: match std::env::var("JWT_SECRET") {
                Ok(s) if s == INSECURE_JWT_DEFAULT => {
                    "set to the insecure placeholder default".to_string()
                }
                Ok(_) => "set but empty".to_string(),
                Err(_) => "not set".to_string(),
            },
            remedy: "a strong random string; it signs every access token",
        });
    }

    if signing_secret().is_none() {
        problems.push(SecretProblem {
            var: "RAISINDB_SIGNING_SECRET",
            detail: match std::env::var("RAISINDB_SIGNING_SECRET") {
                Ok(_) => "set but empty".to_string(),
                Err(_) => "not set".to_string(),
            },
            remedy: "a random 32+ byte string; it signs asset URLs",
        });
    }

    // Validated through the real loader rather than by variable name, so the
    // keyring spellings (`RAISIN_MASTER_KEYS` / `RAISIN_MASTER_KEY_ACTIVE`)
    // and the legacy ones are accepted here exactly as they are at runtime —
    // and a malformed key fails at boot instead of on first decrypt.
    match master_key_with_embedding_fallback() {
        Ok(Some(_)) => {}
        Ok(None) => problems.push(SecretProblem {
            var: "RAISIN_MASTER_KEY",
            detail: "not set (nor RAISIN_MASTER_KEYS, nor EMBEDDING_MASTER_KEY)".to_string(),
            remedy: "a 64-char hex key, or RAISIN_MASTER_KEYS=\"<id>:<64 hex>,…\"",
        }),
        Err(e) => problems.push(SecretProblem {
            var: "RAISIN_MASTER_KEY",
            detail: format!("set but unusable: {e}"),
            remedy: "a 64-char hex key, or RAISIN_MASTER_KEYS=\"<id>:<64 hex>,…\"",
        }),
    }

    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{clear_key_env, ENV_LOCK};

    const HEX_A: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn clear_all() {
        clear_key_env();
        std::env::remove_var("JWT_SECRET");
        std::env::remove_var("RAISINDB_SIGNING_SECRET");
    }

    #[test]
    fn reports_every_missing_secret_in_one_pass() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_all();

        let problems = production_secret_problems();
        let vars: Vec<_> = problems.iter().map(|p| p.var).collect();
        // The regression this guards: the old preflight exited on the first
        // one, so an operator needed three boots to learn three names.
        assert_eq!(
            vars,
            vec!["JWT_SECRET", "RAISINDB_SIGNING_SECRET", "RAISIN_MASTER_KEY"]
        );
        clear_all();
    }

    #[test]
    fn keyring_only_deployment_is_allowed_to_boot() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_all();
        std::env::set_var("JWT_SECRET", "a-real-secret");
        std::env::set_var("RAISINDB_SIGNING_SECRET", "another-real-secret");
        // No RAISIN_MASTER_KEY and no EMBEDDING_MASTER_KEY: the keyring alone.
        std::env::set_var("RAISIN_MASTER_KEYS", format!("1:{HEX_A}"));
        std::env::set_var("RAISIN_MASTER_KEY_ACTIVE", "1");

        assert_eq!(production_secret_problems(), vec![]);
        clear_all();
    }

    #[test]
    fn malformed_master_key_fails_at_boot_not_at_first_use() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_all();
        std::env::set_var("JWT_SECRET", "a-real-secret");
        std::env::set_var("RAISINDB_SIGNING_SECRET", "another-real-secret");
        std::env::set_var("RAISIN_MASTER_KEY", "not-hex");

        let problems = production_secret_problems();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].var, "RAISIN_MASTER_KEY");
        assert!(problems[0].detail.contains("set but unusable"));
        clear_all();
    }

    #[test]
    fn placeholder_and_empty_values_are_not_secrets() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_all();
        std::env::set_var("JWT_SECRET", INSECURE_JWT_DEFAULT);
        std::env::set_var("RAISINDB_SIGNING_SECRET", "");
        std::env::set_var("RAISIN_MASTER_KEY", HEX_A);

        assert!(jwt_secret().is_none());
        assert!(signing_secret().is_none());
        let problems = production_secret_problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].detail.contains("insecure placeholder"));
        assert!(problems[1].detail.contains("empty"));
        clear_all();
    }
}
