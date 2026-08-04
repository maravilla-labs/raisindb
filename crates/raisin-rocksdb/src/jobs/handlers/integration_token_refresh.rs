//! Integration OAuth token-refresh job handler.
//!
//! Handles [`JobType::IntegrationTokenRefresh`]. Scans every repo's
//! `raisin:system` workspace for `raisin:Integration` nodes, finds
//! `connected_accounts` whose OAuth access token is close to expiry, exchanges
//! the stored refresh token for a fresh access token, re-encrypts the token
//! bundle, and writes the node back.
//!
//! Refresh happens entirely in Rust — the refresh token is decrypted here and
//! never crosses into a function sandbox, an API response, or a log line.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use raisin_core::services::node_service::NodeService;
use raisin_crypto::SecretBox;
use raisin_error::Result;
use raisin_locks::LockManagerHandle;
use raisin_models::auth::{oauth_error_detail, AuthContext};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobInfo, JobType};
use raisin_storage::{RepositoryManagementRepository, Storage};
use serde_json::Value;

use crate::RocksDBStorage;

/// Refresh a token this many seconds before it actually expires.
const REFRESH_THRESHOLD_SECS: i64 = 30 * 60;

/// The workspace that holds integration/mount configuration in every repo.
const SYSTEM_WORKSPACE: &str = "raisin:system";

/// Actor stamped on token-refresh writes (kept distinct from the sync engine's
/// `virtual-mount-sync` actor so writeback loop-prevention filters can tell them
/// apart).
const REFRESH_ACTOR: &str = "integration-token-refresh";

/// Length of the periodic refresh bucket used for idempotent scheduling: one
/// job per 10-minute wall-clock window.
const BUCKET_SECS: u64 = 10 * 60;

/// How long one integration's refresh lease is held.
///
/// Bounds a token exchange (one HTTP round trip) plus the node write, and is
/// short enough that a node dying mid-refresh frees the integration well before
/// the next 10-minute sweep.
const REFRESH_LEASE_TTL: Duration = Duration::from_secs(120);

/// Derive the idempotent dedup key for the periodic refresh driver.
///
/// All ticks inside the same 10-minute window collapse to a single key, so at
/// most one `IntegrationTokenRefresh` job is active per window **in this
/// process**.
///
/// It does NOT make the sweep single-fire across a cluster, despite what this
/// comment used to claim: `JobRegistry`'s dedup map is an in-memory `HashMap`
/// with no storage behind it, so every node runs its own sweep. Cross-node
/// safety comes from the per-integration lease in
/// [`IntegrationTokenRefreshHandler::refresh_node`], not from this key.
pub fn token_refresh_dedup_key(now_secs: u64) -> String {
    format!("token-refresh:{}", now_secs / BUCKET_SECS)
}

/// Whether an account whose access token expires at `expires_at_secs` should be
/// refreshed now, given the current time and the pre-expiry threshold.
pub fn is_account_expiring(expires_at_secs: i64, now_secs: i64, threshold_secs: i64) -> bool {
    expires_at_secs <= now_secs + threshold_secs
}

/// Handler for [`JobType::IntegrationTokenRefresh`].
pub struct IntegrationTokenRefreshHandler {
    storage: Arc<RocksDBStorage>,
    http: reqwest::Client,
    /// Cluster-wide serialization of per-integration refreshes. `None` (locks
    /// subsystem disabled) keeps single-node behaviour and warns when
    /// replication is on.
    lock_manager: Option<LockManagerHandle>,
    /// Distinguishes this process in a lease owner string.
    instance_id: String,
}

impl IntegrationTokenRefreshHandler {
    /// Create a new token-refresh handler bound to the given storage.
    pub fn new(storage: Arc<RocksDBStorage>, lock_manager: Option<LockManagerHandle>) -> Self {
        Self {
            storage,
            http: reqwest::Client::new(),
            lock_manager,
            instance_id: nanoid::nanoid!(8),
        }
    }

    /// Best-effort detection of active replication (for the no-locks warning).
    fn replication_active(&self) -> bool {
        self.storage.config.replication_enabled
    }

    /// Scan integrations and refresh any expiring OAuth access tokens.
    pub async fn handle(
        &self,
        job: &JobInfo,
        _context: &raisin_storage::jobs::JobContext,
    ) -> Result<()> {
        let tenant_filter = match &job.job_type {
            JobType::IntegrationTokenRefresh { tenant_id } => tenant_id.clone(),
            _ => None,
        };

        let master_key = match raisin_crypto::master_key_with_embedding_fallback()? {
            Some(k) => k,
            None => {
                tracing::warn!(
                    job_id = %job.id,
                    "IntegrationTokenRefresh: no RAISIN_MASTER_KEY configured; skipping (tokens cannot be decrypted)"
                );
                return Ok(());
            }
        };
        let secret_box = SecretBox::new(&master_key);

        let tenants = match &tenant_filter {
            Some(t) => vec![t.clone()],
            None => crate::management::list_tenants(&self.storage).await?,
        };

        let now_secs = chrono::Utc::now().timestamp();
        let mut refreshed = 0usize;

        for tenant in tenants {
            let repos = match crate::management::list_repositories(&self.storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "token-refresh: failed to list repos");
                    continue;
                }
            };
            for repo in repos {
                match self
                    .refresh_repo(&tenant, &repo, &secret_box, now_secs)
                    .await
                {
                    Ok(n) => refreshed += n,
                    Err(e) => tracing::warn!(
                        tenant = %tenant,
                        repo = %repo,
                        error = %e,
                        "token-refresh: repo scan failed"
                    ),
                }
            }
        }

        if refreshed > 0 {
            tracing::info!(job_id = %job.id, accounts_refreshed = refreshed, "IntegrationTokenRefresh completed");
        }
        Ok(())
    }

    /// Refresh expiring accounts for every integration in one repo. Returns the
    /// number of accounts whose tokens were rotated.
    async fn refresh_repo(
        &self,
        tenant: &str,
        repo: &str,
        secret_box: &SecretBox,
        now_secs: i64,
    ) -> Result<usize> {
        let branch = self
            .storage
            .repository_management()
            .get_repository(tenant, repo)
            .await
            .ok()
            .flatten()
            .map(|r| r.config.default_branch)
            .unwrap_or_else(|| "main".to_string());

        // Captured before `branch` moves into the service below; the lease is
        // scoped per branch, matching where the config node actually lives.
        let branch_for_lease = branch.clone();

        // System context bypasses RLS but stamps our dedicated refresh actor.
        let mut auth = AuthContext::system();
        auth.user_id = Some(REFRESH_ACTOR.to_string());
        let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
            self.storage.clone(),
            tenant.to_string(),
            repo.to_string(),
            branch,
            SYSTEM_WORKSPACE.to_string(),
        )
        .with_auth(auth);

        let integrations = svc.list_by_type("raisin:Integration").await?;
        let mut count = 0usize;
        for node in integrations {
            match self
                .refresh_node(
                    &svc,
                    node,
                    secret_box,
                    now_secs,
                    tenant,
                    repo,
                    &branch_for_lease,
                )
                .await
            {
                Ok(n) => count += n,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "token-refresh: node failed")
                }
            }
        }
        Ok(count)
    }

    /// Refresh all expiring accounts on one integration node, persisting the
    /// node only if at least one account was rotated.
    ///
    /// Holds a per-integration lease for the whole exchange-and-write. Two
    /// things break without it, and both are silent:
    ///
    /// 1. **Refresh-token rotation.** Providers that issue a new refresh token
    ///    on each exchange invalidate the old one. Two nodes presenting the same
    ///    stored refresh token concurrently means one of them wins and the other
    ///    gets `invalid_grant` — and the account ends up disconnected, needing a
    ///    manual reconnect.
    /// 2. **Lost update.** Both nodes read `connected_accounts`, mutate their
    ///    own copy and write the whole node back; the second write silently
    ///    discards the first node's rotated tokens.
    ///
    /// The periodic driver's dedup key does NOT prevent either: the job
    /// registry's dedup map is per-process (see [`token_refresh_dedup_key`]).
    #[allow(clippy::too_many_arguments)]
    async fn refresh_node(
        &self,
        svc: &NodeService<RocksDBStorage>,
        node: Node,
        secret_box: &SecretBox,
        now_secs: i64,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> Result<usize> {
        let lock_key = raisin_locks::scoped_key(
            tenant,
            repo,
            branch,
            &format!("integration-token-refresh:{}", node.id),
        );
        let lease = match &self.lock_manager {
            Some(lm) => {
                let owner = format!("{}:{}", self.instance_id, node.id);
                match lm.try_acquire(&lock_key, &owner, REFRESH_LEASE_TTL).await? {
                    Some(guard) => Some(guard.token),
                    None => {
                        // Another node holds it; its sweep covers this
                        // integration. Doing nothing is correct, not a failure.
                        tracing::debug!(
                            node_path = %node.path,
                            "integration is being refreshed elsewhere; skipping"
                        );
                        return Ok(0);
                    }
                }
            }
            None => {
                if self.replication_active() {
                    tracing::warn!(
                        node_path = %node.path,
                        "locks subsystem disabled while replication is active: OAuth token \
                         refresh is NOT cluster-safe — concurrent refreshes can invalidate a \
                         rotating refresh token and disconnect the account (configure \
                         [locks] backend=redis)"
                    );
                }
                None
            }
        };

        let outcome = self
            .refresh_node_locked(svc, node, secret_box, now_secs)
            .await;

        if let (Some(lm), Some(token)) = (&self.lock_manager, lease) {
            // Best-effort: the lease expires on its own, and failing the whole
            // refresh because a release failed would be worse than the leak.
            let _ = lm.release(&lock_key, token).await;
        }
        outcome
    }

    /// The refresh itself, with the per-integration lease already held.
    async fn refresh_node_locked(
        &self,
        svc: &NodeService<RocksDBStorage>,
        node: Node,
        secret_box: &SecretBox,
        now_secs: i64,
    ) -> Result<usize> {
        let token_url = string_prop(&node.properties, "oauth_config", "token_url");
        let client_id = string_prop(&node.properties, "oauth_config", "client_id");
        let (Some(token_url), Some(client_id)) = (token_url, client_id) else {
            return Ok(0);
        };
        let client_secret = decrypt_client_secret(&node.properties, secret_box);

        let accounts = match node.properties.get("connected_accounts") {
            Some(pv) => match serde_json::to_value(pv) {
                Ok(Value::Array(a)) => a,
                _ => return Ok(0),
            },
            None => return Ok(0),
        };

        // Refreshed token blobs, keyed by account id. Collected from the
        // snapshot WITHOUT holding it for the write: the exchanges below are
        // network round trips taking seconds, and writing this stale node back
        // afterwards would discard any connection an admin added meanwhile.
        // The results are re-applied by id to a freshly-read node instead.
        let mut refreshed: Vec<(String, String, i64)> = Vec::new();

        let mut changed = 0usize;
        for acct in accounts.iter() {
            let Some(account_id) = acct.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let expires_at = acct.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
            if !is_account_expiring(expires_at, now_secs, REFRESH_THRESHOLD_SECS) {
                continue;
            }
            let Some(enc) = acct.get("tokens_encrypted").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(tokens) = secret_box.decrypt_json(enc) else {
                tracing::warn!("token-refresh: could not decrypt account tokens");
                continue;
            };
            let Some(refresh_token) = tokens.get("refresh_token").and_then(|v| v.as_str()) else {
                continue;
            };

            match self
                .exchange_refresh_token(
                    &token_url,
                    &client_id,
                    client_secret.as_deref(),
                    refresh_token,
                )
                .await
            {
                Ok((access_token, new_refresh, expires_in)) => {
                    let refresh_final = new_refresh.unwrap_or_else(|| refresh_token.to_string());
                    let blob = serde_json::json!({
                        "access_token": access_token,
                        "refresh_token": refresh_final,
                    });
                    if let Ok(reenc) = secret_box.encrypt_json(&blob) {
                        refreshed.push((
                            account_id.to_string(),
                            reenc,
                            now_secs + expires_in.max(0),
                        ));
                        changed += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "token-refresh: provider refresh grant failed")
                }
            }
        }

        if changed == 0 {
            return Ok(0);
        }

        // Re-read and patch by id. Anything that changed while we were talking
        // to the provider — a connection added, another removed — survives,
        // because we only touch the entries we actually refreshed. A whole-array
        // write here is what used to silently delete a concurrently-added
        // connection.
        let mut fresh = match svc.get(&node.id).await? {
            Some(n) => n,
            // Connector deleted mid-refresh: nothing to write back.
            None => return Ok(0),
        };
        let mut current = match fresh.properties.get("connected_accounts") {
            Some(pv) => match serde_json::to_value(pv) {
                Ok(Value::Array(a)) => a,
                _ => return Ok(0),
            },
            None => return Ok(0),
        };

        let mut applied = 0usize;
        for (account_id, blob, expires_at) in refreshed {
            let Some(entry) = current
                .iter_mut()
                .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(account_id.as_str()))
            else {
                // Disconnected while we were refreshing it — dropping the new
                // token is correct; re-adding the entry would resurrect it.
                continue;
            };
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("tokens_encrypted".into(), Value::String(blob));
                obj.insert("expires_at".into(), Value::from(expires_at));
                applied += 1;
            }
        }

        if applied == 0 {
            return Ok(0);
        }

        fresh.properties.insert(
            "connected_accounts".to_string(),
            serde_json::from_value::<PropertyValue>(Value::Array(current)).map_err(|e| {
                raisin_error::Error::Validation(format!("connected_accounts re-encode failed: {e}"))
            })?,
        );
        svc.update_node(fresh).await?;
        Ok(applied)
    }

    /// POST the `refresh_token` grant and return `(access_token, new_refresh_token?, expires_in_secs)`.
    ///
    /// Neither the client secret nor either token is ever logged.
    async fn exchange_refresh_token(
        &self,
        token_url: &str,
        client_id: &str,
        client_secret: Option<&str>,
        refresh_token: &str,
    ) -> Result<(String, Option<String>, i64)> {
        let mut form = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ];
        if let Some(secret) = client_secret {
            form.push(("client_secret", secret));
        }

        let resp = self
            .http
            .post(token_url)
            .form(&form)
            .send()
            .await
            .map_err(|e| {
                raisin_error::Error::Backend(format!("token endpoint request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            // As in the OAuth callback: the RFC 6749 §5.2 `error` /
            // `error_description` fields are diagnostic text, not credential
            // material, and are the only way to tell an expired client secret
            // from a revoked grant. Nothing else from the body is read.
            let status = resp.status();
            let detail = oauth_error_detail(&resp.text().await.unwrap_or_default());
            return Err(raisin_error::Error::Backend(if detail.is_empty() {
                format!("token endpoint returned {status}")
            } else {
                format!("token endpoint returned {status} — {detail}")
            }));
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| raisin_error::Error::Backend(format!("token endpoint bad JSON: {e}")))?;

        let access_token = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                raisin_error::Error::Backend("token response missing access_token".into())
            })?
            .to_string();
        let new_refresh = body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let expires_in = body
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3600);

        Ok((access_token, new_refresh, expires_in))
    }
}

/// Read a nested string field: `properties[outer][inner]` where `outer` is an
/// Object-valued property.
fn string_prop(
    properties: &std::collections::HashMap<String, PropertyValue>,
    outer: &str,
    inner: &str,
) -> Option<String> {
    let obj = properties.get(outer)?;
    let json = serde_json::to_value(obj).ok()?;
    json.get(inner)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Decrypt `client_secret_encrypted` (base64 of the standard wire format) if
/// present, returning the plaintext secret. Returns `None` for public clients.
fn decrypt_client_secret(
    properties: &std::collections::HashMap<String, PropertyValue>,
    secret_box: &SecretBox,
) -> Option<String> {
    let enc = match properties.get("client_secret_encrypted")? {
        PropertyValue::String(s) => s,
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(enc).ok()?;
    secret_box.decrypt(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lease — not the dedup key — is what makes the sweep safe on a
    /// cluster. Pinned here because the dedup key LOOKS like it does the job,
    /// and a comment in `main.rs` claimed exactly that for a long time.
    #[tokio::test]
    async fn a_held_lease_makes_the_second_node_skip_the_integration() {
        let locks: raisin_locks::LockManagerHandle =
            Arc::new(raisin_locks::InProcessLockManager::new());
        let key = raisin_locks::scoped_key("t", "r", "main", "integration-token-refresh:node-1");

        // Node A is mid-refresh.
        let held = locks
            .try_acquire(&key, "node-a", REFRESH_LEASE_TTL)
            .await
            .expect("acquire")
            .expect("uncontended");

        // Node B's sweep reaches the same integration and must back off rather
        // than present the same rotating refresh token a second time.
        assert!(
            locks
                .try_acquire(&key, "node-b", REFRESH_LEASE_TTL)
                .await
                .expect("acquire")
                .is_none(),
            "a second node must not refresh an integration already being refreshed"
        );

        // Once A is done, the integration is refreshable again.
        assert!(locks.release(&key, held.token).await.expect("release"));
        assert!(locks
            .try_acquire(&key, "node-b", REFRESH_LEASE_TTL)
            .await
            .expect("acquire")
            .is_some());
    }

    /// Two integrations must refresh in parallel — the lease is per-integration,
    /// not a global sweep lock.
    #[tokio::test]
    async fn different_integrations_do_not_block_each_other() {
        let locks: raisin_locks::LockManagerHandle =
            Arc::new(raisin_locks::InProcessLockManager::new());
        let a = raisin_locks::scoped_key("t", "r", "main", "integration-token-refresh:one");
        let b = raisin_locks::scoped_key("t", "r", "main", "integration-token-refresh:two");

        assert!(locks
            .try_acquire(&a, "n", REFRESH_LEASE_TTL)
            .await
            .unwrap()
            .is_some());
        assert!(locks
            .try_acquire(&b, "n", REFRESH_LEASE_TTL)
            .await
            .unwrap()
            .is_some());
    }

    /// Scoping must isolate tenants: the same integration id in two tenants is
    /// two different integrations.
    #[tokio::test]
    async fn the_lease_is_scoped_per_tenant_and_repo() {
        let locks: raisin_locks::LockManagerHandle =
            Arc::new(raisin_locks::InProcessLockManager::new());
        let one = raisin_locks::scoped_key("t1", "r", "main", "integration-token-refresh:x");
        let two = raisin_locks::scoped_key("t2", "r", "main", "integration-token-refresh:x");

        assert!(locks
            .try_acquire(&one, "n", REFRESH_LEASE_TTL)
            .await
            .unwrap()
            .is_some());
        assert!(
            locks
                .try_acquire(&two, "n", REFRESH_LEASE_TTL)
                .await
                .unwrap()
                .is_some(),
            "one tenant's refresh must not block another's"
        );
    }

    #[test]
    fn dedup_key_buckets_by_ten_minutes() {
        // Two times in the same 10-min window share a key.
        assert_eq!(token_refresh_dedup_key(0), token_refresh_dedup_key(599));
        assert_eq!(token_refresh_dedup_key(600), token_refresh_dedup_key(1199));
        // Adjacent windows differ.
        assert_ne!(token_refresh_dedup_key(599), token_refresh_dedup_key(600));
        assert_eq!(token_refresh_dedup_key(600), "token-refresh:1");
    }

    #[test]
    fn expiring_predicate_honours_threshold() {
        let now = 1_000_000;
        let threshold = 1800;
        // Expires in 10 min -> within the 30-min window -> refresh.
        assert!(is_account_expiring(now + 600, now, threshold));
        // Already expired -> refresh.
        assert!(is_account_expiring(now - 5, now, threshold));
        // Exactly at the threshold boundary -> refresh (inclusive).
        assert!(is_account_expiring(now + threshold, now, threshold));
        // Expires in 1 hour -> outside the window -> leave alone.
        assert!(!is_account_expiring(now + 3600, now, threshold));
    }
}
