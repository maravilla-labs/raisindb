//! Virtual-mount sync engine.
//!
//! The core of the Virtual Nodes feature: keeps external systems mounted into
//! workspaces in sync by invoking adapter functions and materializing their
//! results through the normal transactional write path.
//!
//! Handles two job types:
//! - [`JobType::VirtualMountSyncCheck`] — periodic scan enqueuing due syncs.
//! - [`JobType::VirtualMountSync`] — sync one mount (delta or full).
//!
//! `raisin-rocksdb` cannot depend on `raisin-functions`, so adapters are invoked
//! through an injected [`FunctionExecutorCallback`] (dependency inversion). A
//! per-mount lease lock from `raisin-locks` (when configured) makes syncs
//! cluster-safe; without it the engine falls back to dedup-key-only semantics.

mod adapter;
pub mod check;
mod config;
mod delta;
mod ephemeral;
mod full;
mod materializer;
mod subscription;

#[cfg(test)]
mod tests;

pub use adapter::{
    build_input, build_mount_snapshot, AdapterError, AdapterInvoker, AdapterInvokerHandle,
    Capabilities, FunctionAdapterInvoker,
};
// Credential assembly lives in raisin-models so the sync engine and the HTTP
// connection-test handler cannot drift apart.
pub use config::{
    default_mapping, passes_filters, Change, ChangesPage, ConnectedAccount, ExternalItem,
    IntegrationConfig, ListPage, MappedNode, MountConfig, MountState, SyncConfig, WriteConfig,
    SYNC_ACTOR, SYSTEM_WORKSPACE,
};
pub use materializer::{
    MountScope, NodeMaterializer, RocksDbMaterializer, VirtualMeta, VirtualNodeRef,
};
pub use raisin_models::nodes::integrations::build_credential;

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use raisin_core::services::node_service::NodeService;
use raisin_error::{Error, Result};
use raisin_locks::LockManagerHandle;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::integrations::AccountSelectionError;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{BranchRepository, RepositoryManagementRepository, Storage};
use serde_json::{json, Value};

use crate::RocksDBStorage;

/// Lease TTL for a per-mount sync run.
const SYNC_LEASE_TTL: Duration = Duration::from_secs(600);

/// Handler for the two virtual-mount job types.
pub struct VirtualMountSyncHandler {
    storage: Arc<RocksDBStorage>,
    invoker: Option<AdapterInvokerHandle>,
    materializer: Arc<dyn NodeMaterializer>,
    lock_manager: Option<LockManagerHandle>,
    /// Per-process identity, used to build the lease owner string.
    instance_id: String,
}

impl VirtualMountSyncHandler {
    /// Create a handler. `invoker` is `None` when no function executor is wired
    /// (adapter calls then fail cleanly); `lock_manager` is `None` when the
    /// locks subsystem is disabled (single-node dedup semantics).
    pub fn new(
        storage: Arc<RocksDBStorage>,
        invoker: Option<AdapterInvokerHandle>,
        lock_manager: Option<LockManagerHandle>,
    ) -> Self {
        let materializer = Arc::new(RocksDbMaterializer::new(storage.clone()));
        Self {
            storage,
            invoker,
            materializer,
            lock_manager,
            instance_id: nanoid::nanoid!(8),
        }
    }

    /// Dispatch a virtual-mount job.
    pub async fn handle(&self, job: &JobInfo, context: &JobContext) -> Result<Option<Value>> {
        match &job.job_type {
            JobType::VirtualMountSyncCheck { tenant_id, repo_id } => {
                check::run_check(&self.storage, tenant_id.clone(), repo_id.clone())
                    .await
                    .map(|_| None)
            }
            JobType::VirtualMountSync { mount_id, mode } => self
                .run_sync(job, context, mount_id.clone(), mode.clone())
                .await
                .map(|_| None),
            JobType::VirtualMountSubscriptionRenew { tenant_id } => self
                .run_subscription_renew(tenant_id.clone())
                .await
                .map(|_| None),
            other => Err(Error::Validation(format!(
                "VirtualMountSyncHandler received unexpected job type: {other}"
            ))),
        }
    }

    /// Sync a single mount.
    async fn run_sync(
        &self,
        job: &JobInfo,
        context: &JobContext,
        mount_id: String,
        mode: String,
    ) -> Result<()> {
        let tenant = context.tenant_id.clone();
        let repo = context.repo_id.clone();
        // The job is always enqueued (by `check`) on the repo's config (default)
        // branch: that is the ONLY branch the mount/integration configuration is
        // read from. Materialized virtual nodes go to `mount.target_branch`
        // (resolved below), which may differ.
        let config_branch = if context.branch.is_empty() {
            "main".to_string()
        } else {
            context.branch.clone()
        };

        let svc = self.system_service(&tenant, &repo, &config_branch);
        let Some(mount_node) = svc.get(&mount_id).await? else {
            tracing::warn!(mount_id = %mount_id, "virtual mount node not found; skipping");
            return Ok(());
        };
        let mount = MountConfig::from_node(&mount_node)
            .map_err(|e| Error::Validation(format!("invalid mount {mount_id}: {e}")))?;

        if !mount.enabled {
            // Best-effort push teardown: a disabled mount that still holds a
            // provider subscription should stop receiving pings.
            self.teardown_push(&svc, &tenant, &repo, &config_branch, &mount)
                .await;
            return Ok(());
        }
        if mount.state.status.as_deref() == Some("auth_required") {
            tracing::debug!(mount_id = %mount_id, "mount paused (auth_required); skipping");
            return Ok(());
        }

        // Validate the materialization target branch exists. A misconfigured
        // mount is marked and skipped (non-fatal) rather than failing the job.
        let target_branch = mount.target_branch.clone();
        if self
            .storage
            .branches()
            .get_branch(&tenant, &repo, &target_branch)
            .await?
            .is_none()
        {
            tracing::warn!(
                mount_id = %mount_id,
                target_branch = %target_branch,
                "mount target_branch does not exist; marking misconfigured and skipping"
            );
            let mut state = mount.state.clone();
            state.status = Some("misconfigured".to_string());
            state.last_error = Some(format!("target_branch '{target_branch}' does not exist"));
            let _ = persist_mount_state(
                &self.storage,
                &tenant,
                &repo,
                &config_branch,
                &mount_id,
                &state,
            )
            .await;
            return Ok(());
        }

        let Some(invoker) = self.invoker.clone() else {
            tracing::warn!(mount_id = %mount_id, "no adapter invoker wired; cannot sync");
            return Ok(());
        };

        // Guard against syncing the WRONG connection. Two cases are unsafe:
        //
        //  - Ambiguous: several connections exist and the mount names none. The
        //    old rule took `accounts[0]`, so adding a second connection silently
        //    repointed every unpinned mount at an arbitrary mailbox — wrong data,
        //    no error anywhere.
        //  - NotFound: the mount names a connection that has been disconnected.
        //
        // `NoAccounts` is deliberately NOT an error: an adapter over a public
        // API needs no credential at all, and those mounts must keep syncing.
        // Their credential simply resolves to `None`, exactly as before.
        if let Some(integ_node) = self.load_integration_node(&svc, &mount).await? {
            if let Ok(cfg) = IntegrationConfig::from_node(&integ_node) {
                if let Err(err) = cfg.account_for(mount.account_ref.as_deref()) {
                    if matches!(err, AccountSelectionError::NoAccounts) {
                        tracing::debug!(
                            mount_id = %mount_id,
                            "connector has no connections; syncing without a credential"
                        );
                    } else {
                        tracing::warn!(
                            mount_id = %mount_id,
                            error = %err,
                            "mount does not resolve to a single connection; marking misconfigured"
                        );
                        let mut state = mount.state.clone();
                        state.status = Some("misconfigured".to_string());
                        state.last_error = Some(err.to_string());
                        let _ = persist_mount_state(
                            &self.storage,
                            &tenant,
                            &repo,
                            &config_branch,
                            &mount_id,
                            &state,
                        )
                        .await;
                        return Ok(());
                    }
                }
            }
        }

        // Resolve integration + adapter path + credential + scope + snapshot.
        let CtxParts {
            integ_node_id,
            adapter_path,
            credential,
            mount_snapshot,
            scope,
        } = self
            .build_ctx_parts(&svc, &tenant, &repo, &config_branch, &mount)
            .await?;

        // Acquire the per-mount lease (cluster safety). Keyed on the config
        // branch — that is the identity the periodic scan enqueues against.
        let lock_key = format!("{tenant}\0{repo}\0{config_branch}\0vmount:{mount_id}");
        let owner = format!("{}:{}", self.instance_id, job.id);
        let (fencing_token, guard_token) = match &self.lock_manager {
            Some(lm) => match lm.try_acquire(&lock_key, &owner, SYNC_LEASE_TTL).await? {
                Some(g) => (Some(g.token), Some(g.token)),
                None => {
                    tracing::info!(mount_id = %mount_id, "mount is being synced elsewhere; no-op");
                    return Ok(());
                }
            },
            None => {
                if self.replication_active() {
                    tracing::warn!(
                        mount_id = %mount_id,
                        "locks subsystem disabled while replication is active: virtual-mount \
                         sync is not cluster-safe (configure [locks] backend=redis)"
                    );
                }
                (None, None)
            }
        };

        // `scope` (built by `build_ctx_parts`) targets `mount.target_branch` for
        // materialization; mount/integration config and mount state stay on the
        // config branch (`ctx.config_branch`).
        let mut ctx = SyncCtx {
            storage: self.storage.clone(),
            scope: scope.clone(),
            config_branch: config_branch.clone(),
            mount: mount.clone(),
            adapter_path,
            invoker: invoker.as_ref(),
            materializer: self.materializer.as_ref(),
            lock_manager: self.lock_manager.clone(),
            lock_key: lock_key.clone(),
            credential,
            mount_snapshot,
        };

        // Ephemeral TTL cleanup up front.
        if mount.sync_config.ephemeral {
            if let Some(ttl) = mount.sync_config.ttl_seconds {
                let _ = ephemeral::cleanup_expired(
                    self.materializer.as_ref(),
                    &scope,
                    ttl,
                    Utc::now().timestamp(),
                )
                .await;
            }
        }

        let mut state = mount.state.clone();
        state.last_fencing_token = fencing_token;

        // Resolve capabilities once and cache them on the integration node (on
        // the config branch) so the admin UI can read them without invoking the
        // adapter.
        let capabilities = ctx.resolve_capabilities().await;
        if let Err(e) = self
            .cache_capabilities(
                &tenant,
                &repo,
                &config_branch,
                &integ_node_id,
                &capabilities,
            )
            .await
        {
            tracing::warn!(mount_id = %mount_id, error = %e, "failed to cache capabilities");
        }

        // Write-through honesty: v1 has no writeback implementation, so a mount
        // asking for `write_through` (or whose adapter can't write) is recorded
        // as unsupported and warned once — never fatal, never status-degraded.
        if mount.write_config.wants_write_through() {
            state.writeback_supported = Some(false);
            tracing::warn!(
                mount_id = %mount_id,
                can_write = capabilities.can_write,
                "mount requests write_through writeback but it is not supported \
                 (v1 has no writeback implementation); syncing read-only"
            );
        }

        // Ensure a push subscription exists for webhook/hybrid mounts whose
        // adapter supports push (idempotent: a live subscription is left alone).
        // Runs every sync so a bootstrap sync — enqueued by the check pass for a
        // webhook mount that lacks a subscription — registers push here, and the
        // periodic renew job keeps it alive thereafter. Fully provider-agnostic.
        subscription::ensure(&ctx, &mut state, &capabilities).await;

        // Choose the sync path. Full reconcile is forced on the first sync, an
        // explicit `mode: "full"`, or when the adapter cannot serve a changes
        // feed (`supports_changes: false`) even if a cursor happens to exist.
        // A remap re-materializes everything through the CURRENT mapper and path
        // template, ignoring etags — the migration path for a connector whose
        // mapping changed (a new node type, a new folder hierarchy) while the
        // provider's items did not. It is a full walk by definition: the delta
        // feed only reports what changed upstream, which for a remap is nothing.
        let remap = mode == "remap";
        if remap {
            ctx.scope.force_rewrite = true;
        }
        let use_full = remap
            || mode == "full"
            || state.last_sync_token.is_none()
            || !capabilities.supports_changes;

        // An unfinished backfill resumes on subsequent runs even when the mount
        // is otherwise on the delta path.
        let backfill_pending = !state.backfill_stack.is_empty() || state.backfill_cursor.is_some();

        let result = if use_full {
            full::run(&ctx, &mut state).await
        } else if backfill_pending {
            // DELTA FIRST, then one backfill chunk — in that order, deliberately.
            //
            // Importing a large mailbox takes hours of chunked walking. Running
            // the backfill first (or on its own job) would mean new mail waited
            // for the whole import to finish. Doing the incremental pass first
            // on every run keeps new items arriving within one interval while
            // history fills in behind them, and because both share this run's
            // single lease there is no second writer to race the mount state.
            let delta_outcome = delta::run(&ctx, &mut state).await;
            match delta_outcome {
                Ok(()) => full::run(&ctx, &mut state).await,
                // A failing delta must not be masked by a successful backfill
                // chunk: surface it and leave the resume point untouched.
                Err(e) => Err(e),
            }
        } else {
            delta::run(&ctx, &mut state).await
        };

        let outcome = self.finalize(&ctx, &mut state, result).await;

        // Release the lease.
        if let (Some(lm), Some(token)) = (&self.lock_manager, guard_token) {
            let _ = lm.release(&lock_key, token).await;
        }
        outcome
    }

    /// Apply terminal state (status / failure counters) and persist it.
    async fn finalize(
        &self,
        ctx: &SyncCtx<'_>,
        state: &mut MountState,
        result: std::result::Result<(), AdapterError>,
    ) -> Result<()> {
        // Stamped on EVERY outcome, before the match, so no arm can forget it.
        // This is what the scheduler backs off against; leaving it unset on the
        // failure paths is what let a broken mount retry on every tick forever
        // (see `MountState::last_attempt_at` and `check::is_due`).
        let now = Utc::now().timestamp();
        state.last_attempt_at = Some(now);

        match result {
            Ok(()) => {
                state.status = Some("ok".to_string());
                state.consecutive_failures = 0;
                state.last_error = None;
                state.last_sync_at = Some(now);
                let _ = persist_state(ctx, state).await;
                Ok(())
            }
            Err(AdapterError::AuthExpired) => {
                state.status = Some("auth_required".to_string());
                state.last_error = Some("auth_expired".to_string());
                let _ = persist_state(ctx, state).await;
                Ok(())
            }
            Err(e) => {
                state.consecutive_failures += 1;
                state.last_error = Some(e.to_string());

                // A misconfigured mount gets its own status so the UI can say
                // "fix this" rather than "it is flaky" — the operator action is
                // completely different.
                if matches!(e, AdapterError::Config(_)) {
                    state.status = Some("misconfigured".to_string());
                } else if state.consecutive_failures >= config::DEGRADE_THRESHOLD {
                    state.status = Some("degraded".to_string());
                }
                let _ = persist_state(ctx, state).await;

                // Returning Err makes the JOB layer retry the whole sync (3x)
                // on top of the scheduler's own re-enqueue. That is right for a
                // blip and wrong for anything that cannot succeed: it multiplies
                // a permanent failure into four provider calls per tick. For a
                // non-retryable error the run is complete — the state carries
                // the diagnosis, and `last_attempt_at` backs the schedule off.
                if e.is_retryable() {
                    Err(Error::Backend(format!("virtual mount sync failed: {e}")))
                } else {
                    tracing::warn!(
                        mount_id = %ctx.mount.mount_id,
                        error = %e,
                        status = state.status.as_deref().unwrap_or("degraded"),
                        "virtual mount sync failed permanently; not retrying until the mount is changed"
                    );
                    Ok(())
                }
            }
        }
    }

    /// Decrypt the mount's account tokens into an adapter credential, stripping
    /// the refresh token. Returns `None` if no key/account/tokens are available.
    fn build_credential(
        &self,
        mount: &MountConfig,
        integration: &IntegrationConfig,
    ) -> Option<Value> {
        adapter::build_mount_credential(mount, integration)
    }

    /// Names of the connector's per-connection config fields flagged
    /// `meta.credential: true` — those that belong in the adapter credential
    /// rather than the mount config.
    ///
    /// Resolved through [`NodeTypeResolver`] so `extends`/mixins are honoured.
    /// Returns empty when the connector declares no `connection_config_type`
    /// (every pre-v2 connector), which is exactly the previous behaviour.
    async fn resolve_credential_fields(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        integration: &IntegrationConfig,
    ) -> Vec<String> {
        use raisin_core::services::node_type_resolver::NodeTypeResolver;

        let Some(type_name) = integration.connection_config_type.as_deref() else {
            return Vec::new();
        };
        let resolver = NodeTypeResolver::new(
            self.storage.clone(),
            tenant.to_string(),
            repo.to_string(),
            // Config types live alongside the connector, on the config branch.
            config_branch.to_string(),
        );
        match resolver.resolve(type_name).await {
            Ok(resolved) => resolved
                .resolved_properties
                .iter()
                .filter(|p| {
                    p.meta
                        .as_ref()
                        .and_then(|m| m.get("credential"))
                        .is_some_and(|v| {
                            matches!(
                                v,
                                raisin_models::nodes::properties::PropertyValue::Boolean(true)
                            )
                        })
                })
                .filter_map(|p| p.name.clone())
                .collect(),
            Err(e) => {
                // A connector referencing a type its package never installed is
                // a misconfiguration, not a reason to stop syncing: the mount
                // still works, minus the credential-flagged fields.
                tracing::warn!(
                    config_type = %type_name,
                    error = %e,
                    "could not resolve connection config type; credential fields unavailable"
                );
                Vec::new()
            }
        }
    }

    /// Load a mount's backing `raisin:Integration` node.
    ///
    /// `integration_ref` may be either a path or a node id, so both are tried.
    /// Shared by the pre-flight connection check and [`Self::build_ctx_parts`]
    /// so the two can never disagree about which connector a mount points at.
    async fn load_integration_node(
        &self,
        svc: &NodeService<RocksDBStorage>,
        mount: &MountConfig,
    ) -> Result<Option<raisin_models::nodes::Node>> {
        Ok(svc
            .get_by_path(&mount.integration_ref)
            .await?
            .or(svc.get(&mount.integration_ref).await.unwrap_or_default()))
    }

    /// Resolve everything needed to invoke a mount's adapter: the integration
    /// node id (for capability caching), adapter path, decrypted credential,
    /// mount snapshot, and materialization scope. Shared by the sync run, the
    /// disabled-mount push teardown, and the subscription-renew scan so all
    /// adapter invocations are built identically.
    async fn build_ctx_parts(
        &self,
        svc: &NodeService<RocksDBStorage>,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
    ) -> Result<CtxParts> {
        let Some(integ_node) = self.load_integration_node(svc, mount).await? else {
            return Err(Error::Validation(format!(
                "integration not found: {}",
                mount.integration_ref
            )));
        };
        let mut integration = IntegrationConfig::from_node(&integ_node)
            .map_err(|e| Error::Validation(format!("invalid integration: {e}")))?;
        // Resolve which per-connection config fields belong in the credential
        // rather than the mount config (an IMAP `username`). This needs storage,
        // so it cannot live in the synchronous `from_node` parse; doing it here
        // keeps that parse callable from the replication apply path.
        integration.credential_fields = self
            .resolve_credential_fields(tenant, repo, config_branch, &integration)
            .await;
        let adapter_path = mount
            .adapter_function
            .clone()
            .or_else(|| integration.adapter_function.clone())
            .ok_or_else(|| Error::Validation("no adapter_function configured".to_string()))?;
        let credential = self.build_credential(mount, &integration);
        // The snapshot carries the merged config, so it needs the chosen
        // connection. A selection error is not fatal here — the mount still gets
        // a snapshot (with no per-connection layer) and the missing credential
        // is what surfaces the problem, with `finalize` recording the reason.
        let account = integration.account_for(mount.account_ref.as_deref()).ok();
        let mount_snapshot = build_mount_snapshot(mount, &integration, account);
        let scope = MountScope {
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: mount.target_branch.clone(),
            workspace: mount.target_workspace.clone(),
            mount_id: mount.mount_id.clone(),
            mount_path: mount.mount_path.clone(),
            // Off for every ordinary path. `run_sync` turns it on for a remap;
            // the teardown and renew callers must never rewrite content.
            force_rewrite: false,
        };
        Ok(CtxParts {
            integ_node_id: integ_node.id,
            adapter_path,
            credential,
            mount_snapshot,
            scope,
        })
    }

    /// Best-effort push teardown for a disabled mount: unsubscribe the provider
    /// subscription and clear the push fields. Never fails the caller.
    async fn teardown_push(
        &self,
        svc: &NodeService<RocksDBStorage>,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
    ) {
        if mount.state.push_subscription_id.is_none() {
            return;
        }
        let Some(invoker) = self.invoker.clone() else {
            return;
        };
        let Ok(parts) = self
            .build_ctx_parts(svc, tenant, repo, config_branch, mount)
            .await
        else {
            return;
        };
        let ctx = subscription::adapter_ctx(
            self.storage.clone(),
            parts.scope,
            config_branch.to_string(),
            mount.clone(),
            parts.adapter_path,
            invoker.as_ref(),
            self.materializer.as_ref(),
            parts.credential,
            parts.mount_snapshot,
        );
        let mut state = mount.state.clone();
        subscription::teardown(&ctx, &mut state).await;
        let _ = persist_mount_state(
            &self.storage,
            tenant,
            repo,
            config_branch,
            &mount.mount_id,
            &state,
        )
        .await;
    }

    /// Periodic scan (mirrors `check::run_check`): renew every active push
    /// subscription whose provider expiry is within [`subscription::RENEW_WINDOW_SECS`].
    /// Returns the number of subscriptions renewed.
    async fn run_subscription_renew(&self, tenant_filter: Option<String>) -> Result<usize> {
        let Some(invoker) = self.invoker.clone() else {
            return Ok(0);
        };
        let now = Utc::now().timestamp();
        let tenants = match &tenant_filter {
            Some(t) => vec![t.clone()],
            None => crate::management::list_tenants(&self.storage).await?,
        };

        let mut renewed = 0usize;
        for tenant in tenants {
            let repos = match crate::management::list_repositories(&self.storage, &tenant).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(tenant = %tenant, error = %e, "vmount-renew: list repos failed");
                    continue;
                }
            };
            for repo in repos {
                let config_branch = self
                    .storage
                    .repository_management()
                    .get_repository(&tenant, &repo)
                    .await
                    .ok()
                    .flatten()
                    .map(|r| r.config.default_branch)
                    .unwrap_or_else(|| "main".to_string());
                let svc = self.system_service(&tenant, &repo, &config_branch);
                let mounts = match svc.list_by_type("raisin:VirtualMount").await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(tenant = %tenant, repo = %repo, error = %e, "vmount-renew: list mounts failed");
                        continue;
                    }
                };
                for node in mounts {
                    let Ok(mount) = MountConfig::from_node(&node) else {
                        continue;
                    };
                    if !mount.enabled
                        || mount.state.push_status.as_deref() != Some("active")
                        || !mount
                            .state
                            .push_expires_within(now, subscription::RENEW_WINDOW_SECS)
                    {
                        continue;
                    }
                    let Ok(parts) = self
                        .build_ctx_parts(&svc, &tenant, &repo, &config_branch, &mount)
                        .await
                    else {
                        continue;
                    };
                    let ctx = subscription::adapter_ctx(
                        self.storage.clone(),
                        parts.scope,
                        config_branch.clone(),
                        mount.clone(),
                        parts.adapter_path,
                        invoker.as_ref(),
                        self.materializer.as_ref(),
                        parts.credential,
                        parts.mount_snapshot,
                    );
                    let mut state = mount.state.clone();
                    if subscription::renew(&ctx, &mut state).await {
                        renewed += 1;
                    }
                    let _ = persist_mount_state(
                        &self.storage,
                        &tenant,
                        &repo,
                        &config_branch,
                        &mount.mount_id,
                        &state,
                    )
                    .await;
                }
            }
        }
        Ok(renewed)
    }

    /// Cache the resolved capabilities onto the integration node (on the config
    /// branch) so the admin UI can read them without invoking the adapter.
    async fn cache_capabilities(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        integration_id: &str,
        capabilities: &Capabilities,
    ) -> Result<()> {
        use raisin_models::nodes::properties::PropertyValue;

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant, repo)?;
        tx.set_branch(config_branch)?;
        tx.set_actor(SYNC_ACTOR)?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_message("virtual mount sync: cache capabilities")?;

        let Some(mut node) = tx.get_node(SYSTEM_WORKSPACE, integration_id).await? else {
            return Ok(());
        };
        let caps_value = serde_json::to_value(capabilities)
            .map_err(|e| Error::Validation(format!("capabilities serialize failed: {e}")))?;
        let caps_pv: PropertyValue = serde_json::from_value(caps_value)
            .map_err(|e| Error::Validation(format!("capabilities to PropertyValue failed: {e}")))?;
        node.properties.insert("capabilities".to_string(), caps_pv);
        node.properties.insert(
            "capabilities_checked_at".to_string(),
            PropertyValue::String(Utc::now().to_rfc3339()),
        );
        tx.upsert_node(SYSTEM_WORKSPACE, &node).await?;
        tx.commit().await?;
        Ok(())
    }

    fn system_service(
        &self,
        tenant: &str,
        repo: &str,
        branch: &str,
    ) -> NodeService<RocksDBStorage> {
        NodeService::new_with_context(
            self.storage.clone(),
            tenant.to_string(),
            repo.to_string(),
            branch.to_string(),
            SYSTEM_WORKSPACE.to_string(),
        )
        .with_auth(AuthContext::system())
    }

    /// Best-effort detection of active replication (for the no-locks warning).
    fn replication_active(&self) -> bool {
        self.storage.config.replication_enabled
    }
}

/// Resolved pieces needed to invoke a mount's adapter, produced by
/// [`VirtualMountSyncHandler::build_ctx_parts`].
struct CtxParts {
    /// Integration node id (used to cache resolved capabilities).
    integ_node_id: String,
    adapter_path: String,
    credential: Option<Value>,
    mount_snapshot: Value,
    scope: MountScope,
}

/// Everything a delta/full run needs. Borrows the invoker + materializer.
pub struct SyncCtx<'a> {
    pub storage: Arc<RocksDBStorage>,
    pub scope: MountScope,
    /// The repo's config (default) branch — where the mount/integration config
    /// and the persisted mount state live. Distinct from `scope.branch`, which
    /// is the materialization target branch.
    pub config_branch: String,
    pub mount: MountConfig,
    pub adapter_path: String,
    pub invoker: &'a dyn AdapterInvoker,
    pub materializer: &'a dyn NodeMaterializer,
    pub lock_manager: Option<LockManagerHandle>,
    pub lock_key: String,
    pub credential: Option<Value>,
    pub mount_snapshot: Value,
}

impl SyncCtx<'_> {
    /// Invoke the adapter for one operation.
    pub async fn call(
        &self,
        operation: &str,
        params: Value,
    ) -> std::result::Result<Value, AdapterError> {
        let input = build_input(operation, params, &self.credential, &self.mount_snapshot);
        self.invoker
            .invoke(&self.scope, &self.adapter_path, input)
            .await
    }

    /// Renew the lease between pages during long runs (no-op without a lock).
    pub async fn renew_lease(&self, token: Option<u64>) {
        if let (Some(lm), Some(t)) = (&self.lock_manager, token) {
            let _ = lm.renew(&self.lock_key, t, SYNC_LEASE_TTL).await;
        }
    }

    /// Resolve the adapter's capabilities, falling back conservatively (read
    /// only) when the adapter has no `capabilities` operation, returns a
    /// non-object, or throws. Never fails the sync.
    pub async fn resolve_capabilities(&self) -> Capabilities {
        match self.call("capabilities", json!({})).await {
            Ok(v) => Capabilities::from_adapter_value(&v).unwrap_or_else(|| {
                tracing::warn!(
                    mount_id = %self.scope.mount_id,
                    "adapter returned no usable capabilities; assuming read-only"
                );
                Capabilities::fallback()
            }),
            Err(e) => {
                tracing::warn!(
                    mount_id = %self.scope.mount_id,
                    error = %e,
                    "capabilities probe failed; assuming read-only"
                );
                Capabilities::fallback()
            }
        }
    }
}

/// Map one external item to a node, honoring a custom mapping function.
/// Returns `None` when the mapper filters the item out.
pub async fn map_item(
    ctx: &SyncCtx<'_>,
    item: &ExternalItem,
) -> std::result::Result<Option<MappedNode>, AdapterError> {
    let Some(mapper) = &ctx.mount.mapping_function else {
        return Ok(Some(default_mapping(item)));
    };
    let input = json!({
        "external_item": item,
        "mount": ctx.mount_snapshot,
    });
    let out = ctx.invoker.invoke(&ctx.scope, mapper, input).await?;
    if out.is_null() {
        return Ok(None);
    }
    let node_type = out
        .get("node_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AdapterError::Transient("mapping missing node_type".to_string()))?
        .to_string();
    let name = out
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let properties = out
        .get("properties")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    Ok(Some(MappedNode {
        node_type,
        name,
        properties,
    }))
}

/// Materialize one item (map → upsert) at `rel_path`. Returns whether written.
pub async fn materialize_item(
    ctx: &SyncCtx<'_>,
    item: &ExternalItem,
    rel_path: &str,
) -> std::result::Result<bool, AdapterError> {
    let Some(mapped) = map_item(ctx, item).await? else {
        return Ok(false);
    };
    let virt = VirtualMeta {
        mount_id: ctx.scope.mount_id.clone(),
        external_id: item.external_id.clone(),
        etag: item.etag.clone(),
        synced_at: Utc::now().to_rfc3339(),
    };
    ctx.materializer
        .upsert(&ctx.scope, rel_path, mapped, virt)
        .await
        .map_err(|e| AdapterError::Transient(format!("materialize failed: {e}")))
}

/// Persist the mount's `state` back to its `raisin:system` node, honoring the
/// fencing-token rule: a write whose fencing token is older than the one
/// already stored is skipped (a GC-stalled sync must not clobber a newer
/// cursor). Returns `true` if the state was written.
pub async fn persist_state(ctx: &SyncCtx<'_>, state: &MountState) -> Result<bool> {
    // Mount state lives with the mount config on the CONFIG branch, never on the
    // materialization target branch.
    persist_mount_state(
        &ctx.storage,
        &ctx.scope.tenant,
        &ctx.scope.repo,
        &ctx.config_branch,
        &ctx.scope.mount_id,
        state,
    )
    .await
}

/// Standalone state-persist (also directly unit-testable).
pub async fn persist_mount_state(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    mount_id: &str,
    state: &MountState,
) -> Result<bool> {
    use raisin_models::nodes::properties::PropertyValue;

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(branch)?;
    tx.set_actor(SYNC_ACTOR)?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_message("virtual mount sync: persist state")?;

    let Some(mut node) = tx.get_node(SYSTEM_WORKSPACE, mount_id).await? else {
        return Ok(false);
    };

    // Fencing check: read the currently-stored token.
    let stored_token = node
        .properties
        .get("state")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| v.get("last_fencing_token").and_then(|t| t.as_u64()));
    if let (Some(stored), Some(ours)) = (stored_token, state.last_fencing_token) {
        if stored > ours {
            tracing::warn!(
                mount_id = %mount_id,
                stored,
                ours,
                "skipping mount-state write: stored fencing token is newer (stale sync)"
            );
            return Ok(false);
        }
    }

    let state_value = serde_json::to_value(state)
        .map_err(|e| Error::Validation(format!("state serialize failed: {e}")))?;
    let state_pv: PropertyValue = serde_json::from_value(state_value)
        .map_err(|e| Error::Validation(format!("state to PropertyValue failed: {e}")))?;
    node.properties.insert("state".to_string(), state_pv);
    tx.upsert_node(SYSTEM_WORKSPACE, &node).await?;
    tx.commit().await?;
    Ok(true)
}
