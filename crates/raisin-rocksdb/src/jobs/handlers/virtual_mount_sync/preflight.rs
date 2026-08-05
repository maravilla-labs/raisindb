//! Every reason a sync must not run, checked in one place.
//!
//! The order is load-bearing: the target-branch check precedes the invoker
//! check, which precedes connection selection, so a misconfigured mount is
//! marked as such even on a node with no function executor wired.

use super::*;

/// The outcome of the pre-flight guard sequence: either a run may proceed with
/// a parsed mount and a live invoker, or it must not run at all and says why.
pub(super) enum Preflight {
    Proceed {
        /// Boxed: `MountConfig` embeds the ~40-field `MountState`, so an
        /// unboxed variant would make every `Skip` pay for it.
        mount: Box<MountConfig>,
        invoker: AdapterInvokerHandle,
    },
    /// The reason, as it reaches `skip_result`. `&'static str` on purpose:
    /// every reason is one of a closed set the console renders.
    Skip(&'static str),
}

impl VirtualMountSyncHandler {
    pub(super) async fn preflight(
        &self,
        svc: &NodeService<RocksDBStorage>,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount_id: &str,
    ) -> Result<Preflight> {
        let Some(mount_node) = svc.get(mount_id).await? else {
            tracing::warn!(mount_id = %mount_id, "virtual mount node not found; skipping");
            return Ok(Preflight::Skip("mount_not_found"));
        };
        let mount = MountConfig::from_node(&mount_node)
            .map_err(|e| Error::Validation(format!("invalid mount {mount_id}: {e}")))?;

        if !mount.enabled {
            // Best-effort push teardown: a disabled mount that still holds a
            // provider subscription should stop receiving pings.
            self.teardown_push(&svc, tenant, repo, config_branch, &mount)
                .await;
            return Ok(Preflight::Skip("disabled"));
        }
        if mount.state.paused {
            // NOTE the asymmetry with `!enabled` above: no `teardown_push`.
            // Pausing must not unsubscribe from the provider, or notifications
            // arriving during the pause are lost outright and resuming silently
            // re-registers. Disabling is the destructive one; pausing is not.
            tracing::debug!(mount_id = %mount_id, "mount paused by operator; skipping");
            return Ok(Preflight::Skip("paused"));
        }
        if mount.state.status.as_deref() == Some("auth_required") {
            tracing::debug!(mount_id = %mount_id, "mount paused (auth_required); skipping");
            return Ok(Preflight::Skip("auth_required"));
        }

        // Validate the materialization target branch exists. A misconfigured
        // mount is marked and skipped (non-fatal) rather than failing the job.
        let target_branch = mount.target_branch.clone();
        if self
            .storage
            .branches()
            .get_branch(tenant, repo, &target_branch)
            .await?
            .is_none()
        {
            tracing::warn!(
                mount_id = %mount_id,
                target_branch = %target_branch,
                "mount target_branch does not exist; marking misconfigured and skipping"
            );
            return Ok(self
                .mark_misconfigured(
                    tenant,
                    repo,
                    config_branch,
                    &mount,
                    format!("target_branch '{target_branch}' does not exist"),
                )
                .await);
        }

        let Some(invoker) = self.invoker.clone() else {
            tracing::warn!(mount_id = %mount_id, "no adapter invoker wired; cannot sync");
            return Ok(Preflight::Skip("no_invoker"));
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
                        return Ok(self
                            .mark_misconfigured(
                                tenant,
                                repo,
                                config_branch,
                                &mount,
                                err.to_string(),
                            )
                            .await);
                    }
                }
            }
        }
        Ok(Preflight::Proceed {
            mount: Box::new(mount),
            invoker,
        })
    }

    /// Mark the mount misconfigured, stamp the attempt, persist, and skip.
    ///
    /// The single exit for every pre-flight guard that gives up BEFORE
    /// `finalize` runs. There used to be two of these and they disagreed: the
    /// target-branch guard set only `status` + `last_error`, so it never got
    /// the attempt stamp below and every mount it caught stayed permanently
    /// due.
    async fn mark_misconfigured(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
        error: String,
    ) -> Preflight {
        let mut state = mount.state.clone();
        state.status = Some("misconfigured".to_string());
        state.last_error = Some(error);
        // Stamp the attempt here too. This path returns BEFORE
        // `finalize`, so without it `last_attempt_at` stays null
        // and `is_due` keeps the mount permanently due — the
        // same defect the backoff fix addressed, surviving in
        // the one branch that skips finalize. Harmless for the
        // provider (it fails before any call), but it re-scans
        // and rewrites this node on every 60s tick forever.
        state.last_attempt_at = Some(Utc::now().timestamp());
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if let Err(e) = persist_mount_state(
            &self.storage,
            tenant,
            repo,
            config_branch,
            &mount.mount_id,
            &mut state,
        )
        .await
        {
            tracing::warn!(
                mount_id = %mount.mount_id,
                error = %e,
                "failed to record misconfigured mount state"
            );
        }
        Preflight::Skip("misconfigured")
    }
}

/// A job result for a sync that never ran, naming WHY.
///
/// These paths used to return a bare `Ok(())`, so a mount that was disabled,
/// paused on `auth_required`, locked by another node or missing its adapter all
/// looked identical from the outside: a job that completed with no result and
/// no trace.
pub(super) fn skip_result(reason: &str) -> Value {
    json!({ "outcome": "skipped", "reason": reason })
}
