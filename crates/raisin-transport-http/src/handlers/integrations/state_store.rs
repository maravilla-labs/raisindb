// SPDX-License-Identifier: BSL-1.1

//! Short-lived store for the OAuth `state` parameter.
//!
//! The `state` value ties an outbound-OAuth `/start` request to its eventual
//! `/callback`, carrying the tenant/repo/integration it belongs to and guarding
//! against CSRF. Entries expire after a short TTL and are single-use (consumed
//! on the first successful `/callback`).
//!
//! The backing store is an in-process map. That is correct for a single-node
//! server; a multi-node deployment behind a load balancer must pin the OAuth
//! start/callback pair to the same node (the flow is a one-shot admin action, so
//! this is an acceptable v1 constraint — noted in the plan).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Default lifetime of an OAuth `state` entry.
pub const STATE_TTL_SECS: u64 = 600;

/// What a `state` value resolves to at callback time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateEntry {
    pub tenant_id: String,
    pub repo: String,
    pub integration_path: String,
    /// Unix epoch seconds after which the entry is no longer valid.
    pub expires_at: u64,
}

/// In-process, TTL'd, single-use store for OAuth `state` values.
#[derive(Default)]
pub struct OAuthStateStore {
    entries: Mutex<HashMap<String, StateEntry>>,
}

impl OAuthStateStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Insert a `state` entry that expires `ttl_secs` after `now_secs`.
    pub fn insert(
        &self,
        state: String,
        tenant_id: String,
        repo: String,
        integration_path: String,
        now_secs: u64,
        ttl_secs: u64,
    ) {
        let mut guard = self.entries.lock().unwrap();
        // Opportunistically evict anything already expired so the map cannot
        // grow without bound on abandoned flows.
        guard.retain(|_, e| e.expires_at > now_secs);
        guard.insert(
            state,
            StateEntry {
                tenant_id,
                repo,
                integration_path,
                expires_at: now_secs.saturating_add(ttl_secs),
            },
        );
    }

    /// Consume a `state` entry: remove it and return it only if it exists and
    /// has not expired at `now_secs`. Returns `None` for unknown or expired
    /// state (both indistinguishable to the caller, by design).
    pub fn consume(&self, state: &str, now_secs: u64) -> Option<StateEntry> {
        let mut guard = self.entries.lock().unwrap();
        let entry = guard.remove(state)?;
        if entry.expires_at <= now_secs {
            None
        } else {
            Some(entry)
        }
    }
}

/// Process-global OAuth state store shared by the `/start` and `/callback`
/// handlers.
pub static OAUTH_STATE_STORE: LazyLock<OAuthStateStore> = LazyLock::new(OAuthStateStore::new);

/// Current wall-clock time in unix epoch seconds.
pub fn now_secs() -> u64 {
    chrono::Utc::now().timestamp().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_valid_state() {
        let store = OAuthStateStore::new();
        store.insert(
            "abc".into(),
            "t1".into(),
            "repo1".into(),
            "/integrations/gdrive".into(),
            1000,
            STATE_TTL_SECS,
        );
        let got = store.consume("abc", 1100).expect("should resolve");
        assert_eq!(got.tenant_id, "t1");
        assert_eq!(got.repo, "repo1");
        assert_eq!(got.integration_path, "/integrations/gdrive");
    }

    #[test]
    fn state_is_single_use() {
        let store = OAuthStateStore::new();
        store.insert("abc".into(), "t".into(), "r".into(), "/i".into(), 0, 600);
        assert!(store.consume("abc", 1).is_some());
        // Second consume finds nothing.
        assert!(store.consume("abc", 1).is_none());
    }

    #[test]
    fn expired_state_is_rejected() {
        let store = OAuthStateStore::new();
        store.insert("abc".into(), "t".into(), "r".into(), "/i".into(), 0, 600);
        // now == expires_at (600) -> expired (exclusive upper bound).
        assert!(store.consume("abc", 600).is_none());
    }

    #[test]
    fn unknown_state_is_rejected() {
        let store = OAuthStateStore::new();
        assert!(store.consume("nope", 0).is_none());
    }
}
