// SPDX-License-Identifier: BSL-1.1

//! Serialized read-modify-write of a connector's `connected_accounts`.
//!
//! # The race this closes
//!
//! `connected_accounts` is a single array property on one node, and **four**
//! independent writers mutate it: the OAuth callback (appends a connection), the
//! disconnect handler (removes one), the connections endpoints (add/edit), and
//! the background token-refresh job (rewrites token blobs). Node updates are a
//! plain read-check-write with no optimistic concurrency, so two writers that
//! overlap both read the same array and the second one's write wins wholesale —
//! silently discarding the other's entry.
//!
//! That is not theoretical: the refresh job reads every connector node, performs
//! *network* token exchanges taking seconds, then writes the whole node back. An
//! admin adding a connection inside that window loses it.
//!
//! [`with_accounts_lock`] closes the window by taking a per-connector lease and
//! **re-reading the node inside it**, so the mutation always applies to current
//! state.

use std::time::Duration;

use raisin_models::nodes::Node;

use crate::error::ApiError;
use crate::state::AppState;

/// Lease TTL. Comfortably longer than a node read-modify-write, short enough
/// that a crashed holder frees the connector quickly.
const ACCOUNTS_LOCK_TTL: Duration = Duration::from_secs(15);

/// How many times to retry acquiring before surfacing a conflict.
const ACQUIRE_ATTEMPTS: usize = 5;
/// Backoff between attempts.
const ACQUIRE_BACKOFF: Duration = Duration::from_millis(120);

/// Lock name for a connector's connection list.
fn accounts_lock_key(tenant: &str, repo: &str, integration_path: &str) -> String {
    // Config always lives on `main` (CONFIG_BRANCH), so the branch segment is
    // fixed — two branches never contend for the same connector.
    raisin_locks::scoped_key(
        tenant,
        repo,
        super::CONFIG_BRANCH,
        &format!("integration-accounts:{integration_path}"),
    )
}

/// Run `mutate` against a connector node under an exclusive lease, re-reading
/// the node inside the lock and persisting the result.
///
/// `mutate` receives the freshly-read node and returns any value; it must not
/// perform slow I/O — do network work *before* calling this and pass the results
/// in, so the lease is held only for the read-modify-write.
///
/// When no lock manager is configured the mutation still runs (single-node
/// deployments are the common case and the writers all live in one process);
/// a warning is logged once per call so the exposure is visible rather than
/// silent. Multi-node deployments must configure `[locks]` with the `redis`
/// backend — the same requirement the sync engine already documents.
pub(crate) async fn with_accounts_lock<T, F>(
    state: &AppState,
    tenant: &str,
    repo: &str,
    integration_path: &str,
    actor: &str,
    mutate: F,
) -> Result<T, ApiError>
where
    F: FnOnce(&mut Node) -> Result<T, ApiError>,
{
    let key = accounts_lock_key(tenant, repo, integration_path);
    let owner = format!("http:{actor}:{}", nanoid::nanoid!(8));

    let guard = match &state.lock_manager {
        Some(lm) => {
            let mut acquired = None;
            for attempt in 0..ACQUIRE_ATTEMPTS {
                match lm.try_acquire(&key, &owner, ACCOUNTS_LOCK_TTL).await {
                    Ok(Some(g)) => {
                        acquired = Some(g);
                        break;
                    }
                    // Held elsewhere — back off and retry.
                    Ok(None) => {
                        if attempt + 1 < ACQUIRE_ATTEMPTS {
                            tokio::time::sleep(ACQUIRE_BACKOFF).await;
                        }
                    }
                    // A lock backend outage must not make connectors unmanageable;
                    // fall through to the unlocked path with the same warning.
                    Err(e) => {
                        tracing::warn!(error = %e, "accounts lock backend error; proceeding unlocked");
                        break;
                    }
                }
            }
            if acquired.is_none() {
                return Err(ApiError::new(
                    axum::http::StatusCode::CONFLICT,
                    "CONFLICT",
                    "this connector's connections are being modified; retry in a moment",
                ));
            }
            acquired
        }
        None => {
            tracing::warn!(
                integration_path = %integration_path,
                "locks subsystem disabled: concurrent connection edits and token refresh are \
                 not serialized across nodes (configure [locks] backend=redis for clusters)"
            );
            None
        }
    };

    // Re-read INSIDE the lock. Reading before acquiring would reintroduce the
    // very lost-update window the lease exists to close.
    let svc = super::config_service(state, tenant, repo, actor);
    let result = async {
        let mut node = svc
            .get_by_path(integration_path)
            .await?
            .ok_or_else(|| ApiError::node_not_found(integration_path.to_string()))?;
        let out = mutate(&mut node)?;
        svc.update_node(node).await?;
        Ok::<T, ApiError>(out)
    }
    .await;

    if let (Some(lm), Some(g)) = (&state.lock_manager, &guard) {
        // Best-effort: the lease expires on its own, so a failed release only
        // delays the next writer.
        let _ = lm.release(&g.key, g.token).await;
    }

    result
}
