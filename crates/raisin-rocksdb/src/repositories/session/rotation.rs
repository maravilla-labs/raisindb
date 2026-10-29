//! Atomic refresh-token rotation and the bounded refresh grace.
//!
//! A refresh token is single-use. Rotation is a compare-and-swap on the session's generation
//! (`rotate_refresh_token_if`), serialised by a lock, so of two refreshes of the same token exactly
//! one rotates. The loser, and a late duplicate that arrives within the grace window, are answered
//! by the caller with an access token only; `try_consume_refresh_grace` bounds how many of those a
//! rotation can produce.

use super::SessionRepository;
use crate::keys;
use raisin_error::Result;
use raisin_models::auth::Session;
use raisin_replication::OpType;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::debug;

/// Grace entries older than this are dropped on the next call (the window itself is the caller's).
const GRACE_BOOK_TTL: Duration = Duration::from_secs(300);

/// Process-wide: the HTTP layer builds a `SessionRepository` per request, so the lock and the grace
/// counts cannot live in the repository value. In memory and per node (a cluster node counts its own
/// grace answers; the rotation itself is replicated through the op log).
#[derive(Default)]
struct RefreshGuard {
    /// (tenant, session, generation) -> (first use, answers given)
    grace: HashMap<(String, String, u32), (Instant, u32)>,
}

fn refresh_guard() -> &'static Mutex<RefreshGuard> {
    static GUARD: OnceLock<Mutex<RefreshGuard>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(RefreshGuard::default()))
}

impl SessionRepository {
    /// Rotate the refresh token only if the session is still at `expected_generation`.
    /// Returns `Ok(true)` when this call rotated it, `Ok(false)` when another refresh got there first
    /// (or the session is gone / revoked).
    pub async fn rotate_refresh_token_if(
        &self,
        tenant_id: &str,
        session_id: &str,
        expected_generation: u32,
        new_generation: u32,
        actor: &str,
    ) -> Result<bool> {
        {
            let mut guard = refresh_guard()
                .lock()
                .map_err(|_| raisin_error::Error::storage("refresh guard poisoned".to_string()))?;
            let cf = self.cf_sessions()?;
            let key = keys::session_key(tenant_id, session_id);
            let Some(bytes) = self.db.get_cf(cf, &key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to read session: {}", e))
            })?
            else {
                return Ok(false);
            };
            let mut session: Session = rmp_serde::from_slice(&bytes).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            if session.revoked || session.token_generation != expected_generation {
                return Ok(false);
            }
            session.token_generation = new_generation;
            session.touch();
            let updated = rmp_serde::to_vec_named(&session)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;
            self.db.put_cf(cf, &key, &updated).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to update session: {}", e))
            })?;
            // a new generation starts with a clean grace count
            guard
                .grace
                .retain(|(t, s, _), _| !(t == tenant_id && s == session_id));
        }

        self.operation_capture
            .capture_operation(
                tenant_id.to_string(),
                "_session".to_string(),
                "main".to_string(),
                OpType::RotateRefreshToken {
                    session_id: session_id.to_string(),
                    new_generation,
                },
                actor.to_string(),
                None,
                true,
            )
            .await?;
        debug!(
            "Rotated refresh token for session {} to generation {}",
            session_id, new_generation
        );
        Ok(true)
    }

    /// Count one grace answer for `generation` of the session; `false` once `max` were given.
    pub fn try_consume_refresh_grace(
        &self,
        tenant_id: &str,
        session_id: &str,
        generation: u32,
        max: u32,
    ) -> bool {
        let Ok(mut guard) = refresh_guard().lock() else {
            return false;
        };
        let now = Instant::now();
        guard
            .grace
            .retain(|_, (first, _)| now.duration_since(*first) < GRACE_BOOK_TTL);
        let entry = guard
            .grace
            .entry((tenant_id.to_string(), session_id.to_string(), generation))
            .or_insert((now, 0));
        if entry.1 >= max {
            return false;
        }
        entry.1 += 1;
        true
    }
}
