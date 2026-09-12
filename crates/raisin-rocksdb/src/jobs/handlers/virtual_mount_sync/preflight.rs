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
        let mount = match MountConfig::from_node(&mount_node) {
            Ok(m) => m,
            Err(e) => {
                // NOT an `Err` any more. Failing the job wrote no mount state
                // at all, so the one thing the operator can see — the mount's
                // own status — went on claiming the last successful run while
                // the mount had stopped syncing entirely. A parse failure is a
                // misconfiguration like any other and belongs in the same
                // place as a bad `target_branch`; the difference is only that
                // there is no parsed config to carry it.
                tracing::warn!(
                    mount_id = %mount_id,
                    error = %e,
                    "mount config does not parse; marking misconfigured and skipping"
                );
                if let Err(werr) = misconfig::mark_unparseable_mount(
                    &self.storage,
                    tenant,
                    repo,
                    config_branch,
                    mount_id,
                    &e,
                )
                .await
                {
                    tracing::warn!(
                        mount_id = %mount_id,
                        error = %werr,
                        "failed to record misconfigured mount state"
                    );
                }
                return Ok(Preflight::Skip("misconfigured"));
            }
        };

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
            // Skip — UNLESS the credential has been repaired since it failed.
            //
            // Holding here is right in the steady state: retrying a rejected
            // credential every minute earns a rate limit and fixes nothing. But
            // the only way out of this status is a successful run, and a
            // successful run is exactly what this returns before. So a
            // reconnected account stayed dead, "Sync now" enqueued jobs that
            // were discarded here without a word above debug, and the operator
            // reconnected again. That is the whole "I reconnect and nothing
            // happens" complaint.
            //
            // A credential written AFTER the failure has never been tried. One
            // run is owed to it, and if it is still bad this latches again with
            // a fresh stamp — so a genuinely dead grant still retries once per
            // repair, not once per minute.
            if self.credential_is_newer_than_failure(&svc, &mount).await {
                tracing::info!(
                    mount_id = %mount_id,
                    "the connection was re-authorized after this mount latched \
                     auth_required; clearing it and trying once"
                );
                self.clear_auth_required(tenant, repo, config_branch, &mount)
                    .await;
            } else {
                tracing::debug!(mount_id = %mount_id, "mount paused (auth_required); skipping");
                // Stamp the attempt, exactly as `mark_misconfigured` does and
                // for the same reason: this path returns BEFORE `finalize`, so
                // without it `last_attempt_at` never moves — and `is_due` now
                // uses that stamp to space these re-examinations out. Omitting
                // it turns a ten-minute question into a per-tick one.
                self.stamp_attempt(tenant, repo, config_branch, &mount)
                    .await;
                return Ok(Preflight::Skip("auth_required"));
            }
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
            // A disabled connector is an operator's pause on everything behind
            // it: skip every mount that points at it, the same way a paused
            // mount is skipped, without touching its state or subscription.
            if matches!(
                integ_node.properties.get("enabled"),
                Some(raisin_models::nodes::properties::PropertyValue::Boolean(
                    false
                ))
            ) {
                tracing::debug!(
                    mount_id = %mount_id,
                    integration = %mount.integration_ref,
                    "connector is disabled; skipping"
                );
                return Ok(Preflight::Skip("connector_disabled"));
            }
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
    /// Whether this mount's credential has been written since it latched
    /// `auth_required`.
    ///
    /// Reads `last_refresh_at` off the resolved connection — stamped by BOTH a
    /// successful background refresh and a fresh consent, which is exactly the
    /// set of events that can repair a rejected credential.
    ///
    /// Fails CLOSED: anything unreadable (no integration node, no resolvable
    /// account, no stamp) answers `false` and the mount stays latched. The cost
    /// of a false negative is that an operator reconnects and waits for the next
    /// tick; the cost of a false positive is hammering a provider that is
    /// already rejecting us.
    async fn credential_is_newer_than_failure(
        &self,
        svc: &NodeService<RocksDBStorage>,
        mount: &MountConfig,
    ) -> bool {
        let Some(latched_at) = mount.state.auth_required_at else {
            // Latched by a build that did not stamp it. One clean way out:
            // treat it as un-latchable and let the next failure re-stamp it.
            return false;
        };
        let Ok(Some(integ_node)) = self.load_integration_node(svc, mount).await else {
            return false;
        };
        let Ok(cfg) = IntegrationConfig::from_node(&integ_node) else {
            return false;
        };
        let Ok(account) = cfg.account_for(mount.account_ref.as_deref()) else {
            return false;
        };
        account
            .last_refresh_at
            .is_some_and(|refreshed| refreshed > latched_at)
    }

    /// Record that this mount was looked at, without changing anything else.
    ///
    /// Best-effort: a failed stamp costs one extra re-examination.
    async fn stamp_attempt(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
    ) {
        let mut state = mount.state.clone();
        state.last_attempt_at = Some(Utc::now().timestamp());
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
            tracing::debug!(
                mount_id = %mount.mount_id,
                error = %e,
                "could not stamp the auth_required re-check"
            );
        }
    }

    /// Clear the `auth_required` latch so this run can proceed.
    ///
    /// Best-effort: if the state write fails the run still goes ahead, because
    /// the point is to TRY the repaired credential. A failure here means the
    /// mount latches again on the next rejection, which is the correct
    /// behaviour anyway.
    async fn clear_auth_required(
        &self,
        tenant: &str,
        repo: &str,
        config_branch: &str,
        mount: &MountConfig,
    ) {
        let mut state = mount.state.clone();
        state.status = None;
        state.last_error = None;
        state.auth_required_at = None;
        // The backoff was counting rejections of a credential that no longer
        // exists; a repaired one starts clean or it waits out a delay it did
        // nothing to earn.
        state.consecutive_failures = 0;
        state.retry_after = None;
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
                "could not clear auth_required; the run proceeds anyway"
            );
        }
    }

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

#[cfg(test)]
mod auth_latch_tests {
    use raisin_models::nodes::integrations::ConnectedAccount;

    /// The rule `credential_is_newer_than_failure` applies, isolated from the
    /// storage round trip it needs in production.
    fn unlatches(latched_at: Option<i64>, last_refresh_at: Option<i64>) -> bool {
        let account = ConnectedAccount {
            id: "a1".into(),
            last_refresh_at,
            ..Default::default()
        };
        match latched_at {
            None => false,
            Some(latched) => account
                .last_refresh_at
                .is_some_and(|refreshed| refreshed > latched),
        }
    }

    /// THE fix. A reconnect (or a recovered background refresh) stamps
    /// `last_refresh_at`, and a credential written after the failure has never
    /// been tried — so the mount owes it one run.
    ///
    /// Without this the status is a latch with no exit: escaping it requires a
    /// successful run, and the preflight refuses to run. Production had a mount
    /// whose Test connection reported `auth: valid` sitting on a day-old
    /// `auth_expired`, discarding every "Sync now" in silence.
    #[test]
    fn a_credential_repaired_after_the_failure_unlatches() {
        assert!(unlatches(Some(1_000), Some(1_001)));
    }

    /// The steady state must still hold. Retrying a credential that has not
    /// changed since it was rejected earns a rate limit and fixes nothing.
    #[test]
    fn an_unchanged_credential_stays_latched() {
        assert!(!unlatches(Some(1_000), Some(999)));
        assert!(!unlatches(Some(1_000), Some(1_000)));
    }

    /// Fails CLOSED on missing information: an account with no stamp at all, or
    /// a latch from a build that did not record when it happened. The cost of a
    /// false negative is one tick of delay after a reconnect; the cost of a
    /// false positive is hammering a provider already rejecting us.
    #[test]
    fn missing_information_keeps_the_mount_latched() {
        assert!(!unlatches(Some(1_000), None), "no credential stamp");
        assert!(!unlatches(None, Some(1_001)), "no latch stamp");
        assert!(!unlatches(None, None));
    }
}
