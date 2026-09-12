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

/// How long a connector is left alone after its write-back failed to persist.
///
/// **A write that cannot land does not merely fail to save — it burns the
/// credential.** Each sweep performs a real token exchange at the provider
/// BEFORE attempting to store the result, and a provider that rotates refresh
/// tokens invalidates the stored one the moment it issues the replacement. So a
/// connector whose node write is being refused (a NodeType whose declarations
/// have drifted from the binary is the case observed in production) does not sit
/// still: every retry spends the stored refresh token and throws the
/// replacement away.
///
/// Measured, not hypothetical: one tenant's connector did this once a minute for
/// over two hours, after which reconnecting was the only way back — and the
/// reconnect wrote through the same broken path.
///
/// Backing off does not fix the write; it stops the sweep from destroying a
/// credential it cannot save, and leaves the existing token alone until whatever
/// broke the write is repaired.
const PERSIST_COOLDOWN: Duration = Duration::from_secs(30 * 60);

/// How long the `connected_accounts` write-back lease is held.
///
/// Covers a node read-modify-write and nothing else — every token exchange has
/// already finished by the time it is taken. Deliberately far shorter than
/// [`REFRESH_LEASE_TTL`]: an operator clicking Connect or Disconnect contends
/// for this same lease, and a sweep that held it across its HTTP round trips
/// would answer them `409` for minutes at a time.
const ACCOUNTS_LEASE_TTL: Duration = Duration::from_secs(15);

/// Attempts (and backoff) when the accounts lease is held by an operator's edit.
const ACCOUNTS_ACQUIRE_ATTEMPTS: usize = 5;
const ACCOUNTS_ACQUIRE_BACKOFF: Duration = Duration::from_millis(120);

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

/// What one repo's sweep saw.
///
/// `integrations` is carried so the summary line can distinguish "nothing was
/// due" from "this sweep never reached a connector", and `mcp` is kept SEPARATE
/// from `refreshed` on purpose.
///
/// They used to be one number. `refresh_repo` seeded its counter with the MCP
/// connection refresh and then added connector refreshes onto it, so a sweep
/// reporting `accounts_refreshed=1` several times an hour looked perfectly
/// healthy while the OAuth connector arm refreshed exactly nothing. Two
/// unrelated subsystems sharing one health metric means the louder one hides
/// the broken one.
struct RepoScan {
    integrations: usize,
    /// OAuth connector accounts whose tokens were rotated.
    refreshed: usize,
    /// MCP client connections refreshed — a different subsystem entirely.
    mcp: usize,
}

/// Connectors whose write-back is refusing to persist, and when to try again.
///
/// Separated from the handler so the rule can be tested without a storage
/// engine: what matters is that a refusal suppresses the NEXT exchange, and
/// that a success lifts it. See [`PERSIST_COOLDOWN`] for why retrying is
/// actively harmful rather than merely wasteful.
#[derive(Default)]
struct PersistCooldown {
    entries: std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

impl PersistCooldown {
    /// Time left before this connector may be refreshed again, if any.
    fn remaining(&self, node_id: &str) -> Option<Duration> {
        let mut entries = self.entries.lock().ok()?;
        let until = *entries.get(node_id)?;
        match until.checked_duration_since(std::time::Instant::now()) {
            Some(left) => Some(left),
            None => {
                // Elapsed: drop it, so the map cannot grow without bound across
                // a long-lived process.
                entries.remove(node_id);
                None
            }
        }
    }

    fn mark(&self, node_id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                node_id.to_string(),
                std::time::Instant::now() + PERSIST_COOLDOWN,
            );
        }
    }

    fn clear(&self, node_id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(node_id);
        }
    }
}

/// A successful exchange, ready to be written back.
struct Refreshed {
    /// Re-encrypted `{ access_token, refresh_token }`.
    blob: String,
    expires_at: i64,
}

/// What happened to one connection during a sweep.
///
/// Failures are carried, not dropped. A refresh that fails is the *only*
/// warning an operator will ever get that a grant is dying — by the time the
/// provider's inactivity window closes on the refresh token there is nothing
/// left to diagnose, just a connection that needs reconnecting again and again.
struct AccountOutcome {
    account_id: String,
    /// The `tokens_encrypted` value this outcome was derived from. The
    /// write-back refuses to touch an entry that no longer carries it — see
    /// [`IntegrationTokenRefreshHandler::apply_results`].
    prev_enc: String,
    result: std::result::Result<Refreshed, String>,
}

impl AccountOutcome {
    fn refreshed(account_id: &str, prev_enc: &str, blob: String, expires_at: i64) -> Self {
        Self {
            account_id: account_id.to_string(),
            prev_enc: prev_enc.to_string(),
            result: Ok(Refreshed { blob, expires_at }),
        }
    }

    fn failed(account_id: &str, prev_enc: &str, error: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            prev_enc: prev_enc.to_string(),
            result: Err(error.to_string()),
        }
    }
}

/// Handler for [`JobType::IntegrationTokenRefresh`].
pub struct IntegrationTokenRefreshHandler {
    storage: Arc<RocksDBStorage>,
    http: reqwest::Client,
    /// Connectors whose write-back is currently refusing to persist. See
    /// [`PersistCooldown`].
    persist_broken: PersistCooldown,
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
            persist_broken: PersistCooldown::default(),
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
        let mut integrations_seen = 0usize;
        let mut repos_seen = 0usize;
        let mut mcp_refreshed = 0usize;

        for tenant in tenants {
            let repos = match crate::management::list_repositories(&self.storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "token-refresh: failed to list repos");
                    continue;
                }
            };
            for repo in repos {
                repos_seen += 1;
                match self
                    .refresh_repo(&tenant, &repo, &secret_box, now_secs)
                    .await
                {
                    Ok(scan) => {
                        refreshed += scan.refreshed;
                        integrations_seen += scan.integrations;
                        mcp_refreshed += scan.mcp;
                    }
                    Err(e) => tracing::warn!(
                        tenant = %tenant,
                        repo = %repo,
                        error = %e,
                        "token-refresh: repo scan failed"
                    ),
                }
            }
        }

        // Logged on EVERY sweep, not only when something was rotated.
        //
        // A steady state of "nothing was due" and a sweep that never reaches a
        // connector at all produce the same silence otherwise, and they are the
        // two answers you most need to tell apart when connections keep dying:
        // `integrations_scanned=0` says the sweep ran and found nothing to look
        // after, which is a completely different investigation from
        // `integrations_scanned=12 accounts_refreshed=0` an hour after a
        // connect. One sweep per 10 minutes makes this free.
        tracing::info!(
            job_id = %job.id,
            repos_scanned = repos_seen,
            integrations_scanned = integrations_seen,
            accounts_refreshed = refreshed,
            mcp_connections_refreshed = mcp_refreshed,
            "IntegrationTokenRefresh completed"
        );
        Ok(())
    }

    /// Refresh expiring accounts for every integration in one repo.
    async fn refresh_repo(
        &self,
        tenant: &str,
        repo: &str,
        secret_box: &SecretBox,
        now_secs: i64,
    ) -> Result<RepoScan> {
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

        // Same sweep, same workspace, same lease discipline — one job with two
        // node types rather than two jobs that can drift apart.
        let mcp = crate::jobs::handlers::mcp_connection_refresh::refresh_repo(
            &self.storage,
            &svc,
            self.lock_manager.as_ref(),
            &self.instance_id,
            tenant,
            repo,
            &branch_for_lease,
            secret_box,
            now_secs,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "mcp token refresh failed");
            0
        });

        let integrations = svc.list_by_type("raisin:Integration").await?;
        let mut scan = RepoScan {
            integrations: integrations.len(),
            refreshed: 0,
            mcp,
        };
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
                Ok(n) => scan.refreshed += n,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "token-refresh: node failed")
                }
            }
        }
        Ok(scan)
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
            .refresh_node_locked(svc, node, secret_box, now_secs, tenant, repo)
            .await;

        if let (Some(lm), Some(token)) = (&self.lock_manager, lease) {
            // Best-effort: the lease expires on its own, and failing the whole
            // refresh because a release failed would be worse than the leak.
            let _ = lm.release(&lock_key, token).await;
        }
        outcome
    }

    /// The refresh itself, with the per-integration lease already held.
    #[allow(clippy::too_many_arguments)]
    async fn refresh_node_locked(
        &self,
        svc: &NodeService<RocksDBStorage>,
        node: Node,
        secret_box: &SecretBox,
        now_secs: i64,
        tenant: &str,
        repo: &str,
    ) -> Result<usize> {
        // Refuse to spend a refresh token we already know we cannot store.
        if let Some(retry_in) = self.persist_cooldown_remaining(&node.id) {
            tracing::warn!(
                node_path = %node.path,
                retry_in_secs = retry_in.as_secs(),
                "token-refresh: this connector's last write-back was REFUSED, so refreshing \
                 again would spend the stored refresh token and throw the replacement away; \
                 skipping until the write is fixed"
            );
            return Ok(0);
        }

        // EVERY exit from here on says why.
        //
        // They were all bare `return Ok(0)` / `continue`, and that is how a
        // connector can go months without a single refresh while the sweep
        // reports success every ten minutes. There is no downstream symptom to
        // catch it either: the access token simply expires, the mounts report
        // `auth_expired`, and the operator reconnects — which works, briefly,
        // and teaches them that reconnecting periodically is normal.
        //
        // CONNECTIONS FIRST, config second. The order is the difference between
        // a useful line and 125 of them per sweep.
        //
        // Every connector PACKAGE ships a template node under `/connectors/`
        // with `client_id: ""` — unprovisioned by definition, holding no
        // connections, and never going to refresh anything. Complaining about
        // its config said nothing an operator could act on and buried the one
        // connector that genuinely was misconfigured. A connector with no
        // connections has nothing to refresh; that is the whole statement.
        let accounts = match node.properties.get("connected_accounts") {
            Some(pv) => match serde_json::to_value(pv) {
                Ok(Value::Array(a)) => a,
                other => {
                    tracing::warn!(
                        node_path = %node.path,
                        shape = ?other.map(|v| v.to_string().chars().take(40).collect::<String>()),
                        "token-refresh: connected_accounts is not an array; skipping connector"
                    );
                    return Ok(0);
                }
            },
            None => {
                tracing::debug!(node_path = %node.path, "token-refresh: no connections");
                return Ok(0);
            }
        };
        if accounts.is_empty() {
            tracing::debug!(node_path = %node.path, "token-refresh: no connections");
            return Ok(0);
        }

        let raw_token_url = string_prop(&node.properties, "oauth_config", "token_url");
        let raw_client_id = string_prop(&node.properties, "oauth_config", "client_id");
        let (Some(token_url), Some(client_id)) = (
            non_empty(raw_token_url.clone()),
            non_empty(raw_client_id.clone()),
        ) else {
            // Now genuinely actionable: someone connected an account to a
            // connector that cannot renew it, so it will die at its first
            // expiry. A managed connector whose client the control plane has
            // not minted yet lands here, and so does a half-configured BYO one.
            //
            // Reported as PRESENT-BUT-EMPTY rather than missing, because that is
            // the state a managed connector is actually in and the two want
            // different fixes. An `is_some()` here read `true` for the empty
            // string and said the opposite of what the code had just decided.
            tracing::warn!(
                node_path = %node.path,
                connections = accounts.len(),
                token_url = ?describe_field(raw_token_url.as_deref()),
                client_id = ?describe_field(raw_client_id.as_deref()),
                "token-refresh: connector has connections but no usable oauth_config; \
                 they cannot be renewed and will need reconnecting when they expire"
            );
            return Ok(0);
        };
        let client_secret = decrypt_client_secret(&node.properties, secret_box);

        // One result per account we touched. Collected from the snapshot WITHOUT
        // holding it for the write: the exchanges below are network round trips
        // taking seconds, and writing this stale node back afterwards would
        // discard any connection an admin added meanwhile. The results are
        // re-applied by id to a freshly-read node instead.
        //
        // `prev_enc` travels with each result and is what makes that
        // re-application safe: it identifies the exact token blob the exchange
        // was derived from.
        let mut results: Vec<AccountOutcome> = Vec::new();

        // OAuth connections this connector actually holds, whether or not they
        // were due. Reported so a sweep can say "looked at 3, refreshed 0",
        // which is a different problem from "looked at 0".
        let mut examined = 0usize;
        let mut changed = 0usize;
        for acct in accounts.iter() {
            let Some(account_id) = acct.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let expires_at = acct.get("expires_at").and_then(|v| v.as_i64()).unwrap_or(0);
            if !is_account_expiring(expires_at, now_secs, REFRESH_THRESHOLD_SECS) {
                examined += 1;
                continue;
            }
            let Some(enc) = acct.get("tokens_encrypted").and_then(|v| v.as_str()) else {
                // Due for a refresh and holding no tokens at all. For an OAuth
                // connection that is a broken record, not a credential one.
                if acct.get("auth_kind").and_then(|v| v.as_str()) != Some("config") {
                    tracing::warn!(
                        node_path = %node.path,
                        account_id = %account_id,
                        "token-refresh: connection has no stored tokens; it must be reconnected"
                    );
                }
                continue;
            };
            examined += 1;
            let Ok(tokens) = secret_box.decrypt_json(enc) else {
                // Almost always RAISIN_MASTER_KEY differing from the key the
                // tokens were sealed under. Recorded on the account, not only
                // here: a warn line in a log nobody greps is why an account can
                // sit unrefreshable until its refresh token dies of inactivity
                // and the operator concludes reconnecting is just something you
                // do every couple of weeks.
                tracing::warn!(
                    node_path = %node.path,
                    account_id = %account_id,
                    "token-refresh: could not decrypt account tokens (master key mismatch?)"
                );
                results.push(AccountOutcome::failed(
                    account_id,
                    enc,
                    "stored tokens could not be decrypted (RAISIN_MASTER_KEY mismatch)",
                ));
                continue;
            };
            let Some(refresh_token) = tokens
                .get("refresh_token")
                .and_then(|v| v.as_str())
                // An ABSENT refresh token and an EMPTY one are the same dead
                // grant, and the OAuth callback stores the empty spelling
                // (`unwrap_or_default`) when a provider returns none. Treating
                // only the absent case as missing meant the empty one was posted
                // to the token endpoint on every sweep forever, earning a 400
                // each time.
                .filter(|t| !t.is_empty())
            else {
                // Consent produced no refresh token — the grant can never be
                // kept alive, and the connection will die when the access token
                // expires. Usually a missing `offline_access` / `access_type`.
                results.push(AccountOutcome::failed(
                    account_id,
                    enc,
                    "no refresh token was stored for this connection — reconnect with \
                     offline access granted",
                ));
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
                    match secret_box.encrypt_json(&blob) {
                        Ok(reenc) => {
                            results.push(AccountOutcome::refreshed(
                                account_id,
                                enc,
                                reenc,
                                now_secs + expires_in.max(0),
                            ));
                            changed += 1;
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "token-refresh: could not re-encrypt");
                            results.push(AccountOutcome::failed(
                                account_id,
                                enc,
                                "refreshed tokens could not be encrypted for storage",
                            ));
                        }
                    }
                }
                Err(e) => {
                    // The provider's own words (`invalid_grant`, `consent_required`)
                    // are the only thing that distinguishes a revoked grant from a
                    // broken client secret, and they are already redacted of
                    // credential material by `oauth_error_detail`.
                    tracing::warn!(
                        node_path = %node.path,
                        account_id = %account_id,
                        error = %e,
                        "token-refresh: provider refresh grant failed"
                    );
                    results.push(AccountOutcome::failed(account_id, enc, &e.to_string()));
                }
            }
        }

        if results.is_empty() {
            tracing::debug!(
                node_path = %node.path,
                connections = examined,
                "token-refresh: nothing due on this connector"
            );
            return Ok(0);
        }

        // Re-read and patch by id, under the SAME lease every other writer of
        // this array takes (`raisin_locks::integration_accounts_key`). Anything
        // that changed while we were talking to the provider — a connection
        // added, another removed — survives, because we only touch the entries
        // we handled. A whole-array write here is what used to silently delete a
        // concurrently-added connection; taking a *different* lock from the HTTP
        // writers is what let one of them undo a rotation.
        let accounts_key = raisin_locks::integration_accounts_key(tenant, repo, &node.path);
        let lease = self.acquire_accounts_lease(&accounts_key, &node.path).await;

        let applied = self
            .apply_results(svc, &node, results, now_secs, changed)
            .await;

        if let (Some(lm), Some(token)) = (&self.lock_manager, lease) {
            let _ = lm.release(&accounts_key, token).await;
        }
        applied
    }

    /// Acquire the connector's `connected_accounts` lease, retrying briefly.
    ///
    /// Returns `None` both when there is no lock manager and when the lease
    /// could not be taken. Proceeding unlocked is the right call for the second
    /// case too: the alternative is throwing away a completed token exchange —
    /// and with it the rotated refresh token the provider has already committed
    /// to — which is strictly worse than a narrow lost-update window that the
    /// per-entry `prev_enc` guard below already covers for the field that
    /// matters.
    async fn acquire_accounts_lease(&self, key: &str, node_path: &str) -> Option<u64> {
        let lm = self.lock_manager.as_ref()?;
        let owner = format!("refresh:{}", self.instance_id);
        for attempt in 0..ACCOUNTS_ACQUIRE_ATTEMPTS {
            match lm.try_acquire(key, &owner, ACCOUNTS_LEASE_TTL).await {
                Ok(Some(guard)) => return Some(guard.token),
                Ok(None) => {
                    if attempt + 1 < ACCOUNTS_ACQUIRE_ATTEMPTS {
                        tokio::time::sleep(ACCOUNTS_ACQUIRE_BACKOFF).await;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "accounts lease backend error; proceeding unlocked");
                    return None;
                }
            }
        }
        tracing::warn!(
            node_path = %node_path,
            "connections are being edited; writing back refreshed tokens unlocked"
        );
        None
    }

    /// Apply the collected outcomes to a freshly-read node. Returns how many
    /// tokens were actually rotated (failures are recorded, not counted).
    async fn apply_results(
        &self,
        svc: &NodeService<RocksDBStorage>,
        node: &Node,
        results: Vec<AccountOutcome>,
        now_secs: i64,
        changed: usize,
    ) -> Result<usize> {
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

        let Patched { applied, touched } = patch_accounts(&mut current, results, now_secs);
        if touched == 0 {
            return Ok(0);
        }

        fresh.properties.insert(
            "connected_accounts".to_string(),
            serde_json::from_value::<PropertyValue>(Value::Array(current)).map_err(|e| {
                raisin_error::Error::Validation(format!("connected_accounts re-encode failed: {e}"))
            })?,
        );

        match svc.update_node(fresh).await {
            Ok(_) => {
                self.clear_persist_cooldown(&node.id);
                debug_assert!(applied <= changed);
                Ok(applied)
            }
            Err(e) => {
                // The tokens were already rotated AT THE PROVIDER by the
                // exchange above; failing to store them means the copy we hold
                // is now the dead one. Say that plainly — the write error alone
                // reads like a retryable hiccup, and the sweep retrying it is
                // what turned a schema mismatch into a destroyed credential.
                tracing::error!(
                    node_path = %node.path,
                    accounts = applied,
                    error = %e,
                    "token-refresh: tokens were refreshed at the provider but COULD NOT BE \
                     SAVED; the stored refresh token may now be dead. Backing off this \
                     connector until the write is fixed"
                );
                self.mark_persist_broken(&node.id);
                Err(e)
            }
        }
    }

    /// How long is left on a connector's persist cooldown, if it has one.
    fn persist_cooldown_remaining(&self, node_id: &str) -> Option<Duration> {
        self.persist_broken.remaining(node_id)
    }

    fn mark_persist_broken(&self, node_id: &str) {
        self.persist_broken.mark(node_id);
    }

    fn clear_persist_cooldown(&self, node_id: &str) {
        self.persist_broken.clear(node_id);
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

/// What [`patch_accounts`] did.
struct Patched {
    /// Connections whose tokens were actually rotated.
    applied: usize,
    /// Connections whose entry was modified at all (a recorded failure counts).
    touched: usize,
}

/// Apply refresh outcomes to a freshly-read `connected_accounts` array.
///
/// Pure, and separated from the storage round trip on purpose: the two rules
/// below are the whole safety of the write-back, and neither is observable from
/// a passing sweep — getting one wrong produces a connection that works for a
/// fortnight and then does not.
fn patch_accounts(current: &mut [Value], results: Vec<AccountOutcome>, now_secs: i64) -> Patched {
    let mut applied = 0usize;
    let mut touched = 0usize;

    for outcome in results {
        let Some(entry) = current
            .iter_mut()
            .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(outcome.account_id.as_str()))
        else {
            // Disconnected while we were refreshing it — dropping the new token
            // is correct; re-adding the entry would resurrect it.
            continue;
        };

        // Whoever wrote this entry since we read it knows something we do not:
        // an operator completed a fresh consent, and the provider invalidated
        // the refresh token our exchange was based on when it issued theirs.
        // Writing our result would replace a live grant with a dead one — the
        // exact failure this whole path exists to prevent.
        if entry.get("tokens_encrypted").and_then(|v| v.as_str()) != Some(outcome.prev_enc.as_str())
        {
            tracing::info!(
                account_id = %outcome.account_id,
                "connection was re-authorized during the refresh; keeping the newer tokens"
            );
            continue;
        }

        let Some(obj) = entry.as_object_mut() else {
            continue;
        };
        touched += 1;
        match outcome.result {
            Ok(Refreshed { blob, expires_at }) => {
                obj.insert("tokens_encrypted".into(), Value::String(blob));
                obj.insert("expires_at".into(), Value::from(expires_at));
                obj.insert("last_refresh_at".into(), Value::from(now_secs));
                // The grant is healthy again; a stale error would keep the
                // console telling an operator to reconnect something that just
                // refreshed itself.
                obj.remove("last_refresh_error");
                obj.remove("last_refresh_error_at");
                applied += 1;
            }
            Err(error) => {
                obj.insert("last_refresh_error".into(), Value::String(error));
                obj.insert("last_refresh_error_at".into(), Value::from(now_secs));
            }
        }
    }

    Patched { applied, touched }
}

/// How a config field is populated, for a log line that must not lie.
///
/// "missing" and "empty" are different faults with different fixes — an
/// unprovisioned managed connector has an EMPTY `client_id`, not an absent one —
/// and collapsing them into a bool reported `true` for a value the code had just
/// rejected.
fn describe_field(value: Option<&str>) -> &'static str {
    match value {
        None => "missing",
        Some("") => "empty",
        Some(_) => "set",
    }
}

/// Treat an empty string as absent.
///
/// A managed connector ships `client_id: ""` until the control plane mints one,
/// and `Some("")` sails through every `is_some()` check — it was posted to the
/// token endpoint as a real client id, which the broker answered
/// `invalid_client` for, forever.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
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

    fn account(id: &str, enc: &str) -> Value {
        serde_json::json!({ "id": id, "tokens_encrypted": enc, "expires_at": 1 })
    }

    /// The HTTP surface and this job must contend for ONE lock over
    /// `connected_accounts`. They used to build different keys, so an operator's
    /// edit and a sweep could interleave freely — and the loser was whichever
    /// rotation got reverted, which the provider then answered `invalid_grant`
    /// for ever after.
    #[tokio::test]
    async fn an_operators_edit_blocks_the_sweeps_write_back() {
        let locks: raisin_locks::LockManagerHandle =
            Arc::new(raisin_locks::InProcessLockManager::new());
        // Exactly what `accounts_lock.rs` in raisin-transport-http takes.
        let key = raisin_locks::integration_accounts_key("t", "r", "/connectors/ms-graph");

        let held = locks
            .try_acquire(&key, "http:integration-connections", ACCOUNTS_LEASE_TTL)
            .await
            .expect("acquire")
            .expect("uncontended");

        assert!(
            locks
                .try_acquire(&key, "refresh:node-a", ACCOUNTS_LEASE_TTL)
                .await
                .expect("acquire")
                .is_none(),
            "the sweep must wait for an in-flight connection edit"
        );

        assert!(locks.release(&key, held.token).await.expect("release"));
        assert!(locks
            .try_acquire(&key, "refresh:node-a", ACCOUNTS_LEASE_TTL)
            .await
            .expect("acquire")
            .is_some());
    }

    /// The key is per connector, not per connector-and-branch-and-whatever: both
    /// sides have to derive it from the same three inputs or they never collide.
    #[test]
    fn the_accounts_key_is_stable_and_scoped() {
        let a = raisin_locks::integration_accounts_key("t", "r", "/c/ms");
        assert_eq!(a, raisin_locks::integration_accounts_key("t", "r", "/c/ms"));
        assert_ne!(
            a,
            raisin_locks::integration_accounts_key("t2", "r", "/c/ms")
        );
        assert_ne!(a, raisin_locks::integration_accounts_key("t", "r", "/c/g"));
    }

    /// A successful refresh writes the new blob AND clears any recorded failure,
    /// so a connection that heals stops being reported as broken.
    #[test]
    fn a_success_rotates_the_blob_and_clears_the_error() {
        let mut current = vec![serde_json::json!({
            "id": "a1",
            "tokens_encrypted": "OLD",
            "expires_at": 1,
            "last_refresh_error": "invalid_grant",
            "last_refresh_error_at": 5,
        })];
        let out = patch_accounts(
            &mut current,
            vec![AccountOutcome::refreshed("a1", "OLD", "NEW".into(), 9_000)],
            1_000,
        );

        assert_eq!((out.applied, out.touched), (1, 1));
        assert_eq!(current[0]["tokens_encrypted"], "NEW");
        assert_eq!(current[0]["expires_at"], 9_000);
        assert_eq!(current[0]["last_refresh_at"], 1_000);
        assert!(current[0].get("last_refresh_error").is_none());
        assert!(current[0].get("last_refresh_error_at").is_none());
    }

    /// A FAILURE must be persisted. It is the only warning anyone gets that a
    /// grant is dying; dropping it (as the old code did — it returned early
    /// whenever nothing had been refreshed) is what leaves an operator
    /// reconnecting on a schedule with no idea why.
    #[test]
    fn a_failure_is_recorded_without_touching_the_tokens() {
        let mut current = vec![account("a1", "OLD")];
        let out = patch_accounts(
            &mut current,
            vec![AccountOutcome::failed("a1", "OLD", "invalid_grant")],
            1_000,
        );

        assert_eq!(
            (out.applied, out.touched),
            (0, 1),
            "a failure is not a rotation"
        );
        assert_eq!(current[0]["tokens_encrypted"], "OLD");
        assert_eq!(current[0]["last_refresh_error"], "invalid_grant");
        assert_eq!(current[0]["last_refresh_error_at"], 1_000);
        assert!(current[0].get("last_refresh_at").is_none());
    }

    /// THE regression. A fresh consent landing mid-exchange invalidates the
    /// refresh token this sweep's result was derived from, so writing that
    /// result back would replace a live grant with a dead one — and the symptom
    /// is not an error, it is a connector that needs reconnecting again in a
    /// fortnight.
    #[test]
    fn a_reauthorization_during_the_exchange_wins() {
        let mut current = vec![account("a1", "FROM-A-FRESH-CONSENT")];
        let out = patch_accounts(
            &mut current,
            vec![AccountOutcome::refreshed(
                "a1",
                "OLD",
                "DERIVED-FROM-OLD".into(),
                9_000,
            )],
            1_000,
        );

        assert_eq!((out.applied, out.touched), (0, 0));
        assert_eq!(
            current[0]["tokens_encrypted"], "FROM-A-FRESH-CONSENT",
            "the newer grant must survive the sweep"
        );
    }

    /// Same guard, failure side: a stale failure must not be stamped onto a
    /// connection someone has just successfully reconnected.
    #[test]
    fn a_stale_failure_is_not_stamped_on_a_reconnected_account() {
        let mut current = vec![account("a1", "FRESH")];
        let out = patch_accounts(
            &mut current,
            vec![AccountOutcome::failed("a1", "OLD", "invalid_grant")],
            1_000,
        );

        assert_eq!(out.touched, 0);
        assert!(current[0].get("last_refresh_error").is_none());
    }

    /// A connection disconnected mid-sweep must not come back.
    #[test]
    fn a_disconnected_account_is_not_resurrected() {
        let mut current = vec![account("other", "X")];
        let out = patch_accounts(
            &mut current,
            vec![AccountOutcome::refreshed(
                "gone",
                "OLD",
                "NEW".into(),
                9_000,
            )],
            1_000,
        );

        assert_eq!((out.applied, out.touched), (0, 0));
        assert_eq!(current.len(), 1);
    }

    /// A connector whose write-back was refused must be LEFT ALONE, not retried.
    ///
    /// The retry is what does the damage: each one performs a real token
    /// exchange, rotating the refresh token at the provider, and then fails to
    /// store the replacement. Production ran that loop once a minute for over
    /// two hours and the credential did not survive it.
    #[test]
    fn a_refused_write_back_suppresses_the_next_exchange() {
        let cooldown = PersistCooldown::default();

        assert!(cooldown.remaining("node-1").is_none());
        cooldown.mark("node-1");
        assert!(
            cooldown.remaining("node-1").is_some(),
            "a refused write must stop the next refresh from spending the token"
        );
        assert!(
            cooldown.remaining("node-2").is_none(),
            "one broken connector must not pause every other"
        );

        cooldown.clear("node-1");
        assert!(
            cooldown.remaining("node-1").is_none(),
            "a successful write must lift the cooldown"
        );
    }

    /// "missing" and "empty" are different faults wanting different fixes, and
    /// the bool that preceded this reported `true` for a value the code had just
    /// rejected — production printed `has_client_id=true` on the very connector
    /// it was skipping for having no client id.
    #[test]
    fn a_field_report_distinguishes_missing_from_empty() {
        assert_eq!(describe_field(None), "missing");
        assert_eq!(describe_field(Some("")), "empty");
        assert_eq!(describe_field(Some("abc")), "set");
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
